use crate::{
    cache::{BoundedCache, CacheLookup, CacheStorage},
    cookies::CookieJar,
    policy::{
        sanitize_filename, BrowserPolicy, DownloadPolicy, Origin, OriginPolicy, PolicyViolation,
        ProxyPolicy, ProxyRoute, ResourceIntent,
    },
    wire::{ControlMessage, FetchRequest, FetchResponsePayload, FLAG_CACHE_HIT, OP_FETCH_RESP},
};
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use dunit_http_core::{Header, Method, Response, Url};

struct MemCacheStorage {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemCacheStorage {
    fn new() -> Self {
        Self {
            files: BTreeMap::new(),
        }
    }
}

impl CacheStorage for MemCacheStorage {
    fn list_files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        self.files.get(name).cloned()
    }

    fn write_atomic(&mut self, name: &str, data: &[u8]) -> bool {
        self.files.insert(String::from(name), data.to_vec());
        true
    }

    fn remove_file(&mut self, name: &str) {
        self.files.remove(name);
    }
}

#[test]
fn origin_proxy_download_and_redirect_policies() {
    let mut policy = BrowserPolicy::default();
    let https_url = Url::parse("https://docs.example.test/files/report.txt").unwrap();
    assert_eq!(
        policy.check_request(Method::Get, &https_url, ResourceIntent::Document),
        Ok(ProxyRoute::Direct)
    );

    let http_url = Url::parse("http://docs.example.test/").unwrap();
    assert_eq!(
        policy.check_request(Method::Get, &http_url, ResourceIntent::Document),
        Err(PolicyViolation::BlockedScheme)
    );
    assert_eq!(
        policy.origins.check_redirect(&https_url, &http_url),
        Err(PolicyViolation::DowngradeDenied)
    );

    let meta_url = Url::parse("https://169.254.169.254/latest/meta-data/").unwrap();
    assert_eq!(
        policy.origins.check_url(&meta_url, ResourceIntent::Document),
        Err(PolicyViolation::BlockedHost)
    );

    let priv_url = Url::parse("https://192.168.1.50/admin").unwrap();
    assert_eq!(
        policy.origins.check_url(&priv_url, ResourceIntent::Document),
        Err(PolicyViolation::PrivateAddressDenied)
    );

    policy.proxy = ProxyPolicy {
        route: ProxyRoute::HttpForward {
            host: String::from("proxy.internal"),
            port: 3128,
        },
        no_proxy: vec![String::from("*.local")],
    };
    assert_eq!(
        policy.proxy.resolve("cdn.example.com"),
        Ok(ProxyRoute::HttpForward {
            host: String::from("proxy.internal"),
            port: 3128
        })
    );
    assert_eq!(policy.proxy.resolve("dev.local"), Ok(ProxyRoute::Direct));

    let dl = DownloadPolicy {
        max_download_bytes: 1024,
        max_inline_bytes: 256,
        block_executable_inline: true,
    };
    assert_eq!(sanitize_filename("../../etc/../evil\0file.sh"), "evilfile.sh");
    let exe_headers = vec![Header {
        name: String::from("Content-Type"),
        value: String::from("application/x-elf"),
    }];
    assert_eq!(
        dl.classify(&https_url, &exe_headers, 128, ResourceIntent::Document),
        Err(PolicyViolation::ExecutableDownloadOnly)
    );
    let dl_ok = dl
        .classify(&https_url, &exe_headers, 128, ResourceIntent::Download)
        .unwrap();
    assert!(dl_ok.is_download);
    assert_eq!(dl_ok.filename.as_deref(), Some("report.txt"));

    let o1 = Origin::from_url(&https_url);
    assert_eq!(o1.canonical(), "https://docs.example.test");
}

#[test]
fn bounded_atomic_cache_eviction_and_corruption_recovery() {
    let storage = MemCacheStorage::new();
    let mut cache = BoundedCache::open(storage, 2, 512, 600);
    let url1 = Url::parse("https://https.du.net.test/a").unwrap();
    let url2 = Url::parse("https://https.du.net.test/b").unwrap();
    let url3 = Url::parse("https://https.du.net.test/c").unwrap();

    let resp = Response {
        status: 200,
        reason: String::from("OK"),
        url: url1.clone(),
        headers: vec![Header {
            name: String::from("Cache-Control"),
            value: String::from("max-age=60"),
        }],
        body: vec![b'x'; 80],
    };
    assert!(cache.store(Method::Get, &url1, &resp, 1000));
    assert!(matches!(cache.lookup(Method::Get, &url1, 1020), CacheLookup::Hit(_)));

    let slot1 = BoundedCache::<MemCacheStorage>::slot_name(&BoundedCache::<MemCacheStorage>::cache_key(Method::Get, &url1));
    cache.storage_mut().files.get_mut(&slot1).unwrap()[12] ^= 0xff;
    assert_eq!(cache.lookup(Method::Get, &url1, 1020), CacheLookup::MissCorrupt);
    assert_eq!(cache.stats().corrupt_repaired, 1);

    assert!(cache.store(Method::Get, &url1, &resp, 1001));
    assert!(cache.store(Method::Get, &url2, &resp, 1002));
    assert!(cache.store(Method::Get, &url3, &resp, 1003));
    assert_eq!(cache.stats().active_entries, 2);
    assert!(cache.stats().evictions >= 1);
}

#[test]
fn cookie_jar_security_quotas_and_persistence() {
    let mut jar = CookieJar::new(2, 4);
    let https_u = Url::parse("https://app.example.test/account/profile").unwrap();
    let http_u = Url::parse("http://app.example.test/account/profile").unwrap();
    let headers = vec![
        Header {
            name: String::from("Set-Cookie"),
            value: String::from("sid=abc123; Path=/account; Secure; HttpOnly; Max-Age=300"),
        },
        Header {
            name: String::from("Set-Cookie"),
            value: String::from("theme=dark; Max-Age=60"),
        },
        Header {
            name: String::from("Set-Cookie"),
            value: String::from("bad=nope; Domain=evil.test"),
        },
    ];
    assert!(jar.apply_response_headers(&https_u, &headers, 10_000));
    assert_eq!(jar.len(), 2);
    assert_eq!(
        jar.cookie_header_for(&https_u, 10_010).as_deref(),
        Some("sid=abc123; theme=dark")
    );
    assert_eq!(
        jar.cookie_header_for(&http_u, 10_010).as_deref(),
        Some("theme=dark")
    );

    let mut blob = jar.encode();
    let mut reloaded = CookieJar::new(2, 4);
    assert!(reloaded.load_blob(&blob, 10_020));
    assert_eq!(reloaded.len(), 2);

    blob[15] ^= 0x55;
    assert!(!reloaded.load_blob(&blob, 10_020));
    assert_eq!(reloaded.corrupt_reset_count, 1);
    assert!(reloaded.is_empty());

    let origin_pol = OriginPolicy::default();
    assert!(!origin_pol.allow_http);
}

#[test]
fn ipc_control_and_shared_payload_roundtrip() {
    let mut ctrl = ControlMessage::empty(OP_FETCH_RESP, 42);
    ctrl.status_or_error = 200;
    ctrl.shm_handle = 7;
    ctrl.payload_len = 512;
    ctrl.flags = FLAG_CACHE_HIT;
    ctrl.summary = String::from("TLSv1.3 cache-hit");
    let raw = ctrl.encode();
    let decoded = ControlMessage::decode(&raw).unwrap();
    assert_eq!(decoded, ctrl);

    let req = FetchRequest {
        method: Method::Get,
        intent: ResourceIntent::Document,
        bypass_cache: true,
        url: String::from("https://https.du.net.test/viewer"),
        headers: vec![Header {
            name: String::from("Accept"),
            value: String::from("text/plain"),
        }],
        body: Vec::new(),
    };
    assert_eq!(FetchRequest::decode(&req.encode()).unwrap(), req);

    let resp = FetchResponsePayload {
        status: 200,
        final_url: req.url,
        content_type: String::from("text/plain"),
        download_filename: None,
        tls_info: String::from("TLSv1.3 ChaCha20-Poly1305 CN=https.du.net.test"),
        detail: String::from("ok"),
        headers: req.headers,
        body: b"hello browser viewer".to_vec(),
    };
    assert_eq!(FetchResponsePayload::decode(&resp.encode()).unwrap(), resp);
}

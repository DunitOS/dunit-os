/* Opt-in dual-stack (IPv4 + IPv6) socket translation for Dunit musl. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <dunit/posix-sockets.h>
#include <dunit/rights.h>
#include <dunit/socket.h>
#include <dunit/syscall.h>

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

enum {
    OPEN_TIMEOUT_MS = 5000,
    DNS_TIMEOUT_MS = 15000,
    MAX_TRACKED_FDS = 1024,
    MAX_DNS_WIRE_LEN = 2 + 16 * 22,
};

struct sock_state {
    uint8_t active;
    uint8_t nonblocking;
    int domain;
    int sock_type;
    int protocol;
    uint64_t rcv_timeout_ms;
    uint64_t snd_timeout_ms;
};

static uint32_t selected_grant;
static volatile int state_lock;
static struct sock_state fd_states[MAX_TRACKED_FDS];

static void lock_states(void)
{
    while (__atomic_exchange_n(&state_lock, 1, __ATOMIC_ACQUIRE) != 0)
        __asm__ volatile("pause" ::: "memory");
}

static void unlock_states(void)
{
    __atomic_store_n(&state_lock, 0, __ATOMIC_RELEASE);
}

static long native_call(long number, uintptr_t first, uintptr_t second, uintptr_t third,
                        uintptr_t fourth, uintptr_t fifth, uintptr_t sixth)
{
    register uintptr_t fourth_register __asm__("r10") = fourth;
    register uintptr_t fifth_register __asm__("r8") = fifth;
    register uintptr_t sixth_register __asm__("r9") = sixth;
    long result;
    __asm__ volatile(
        "syscall"
        : "=a"(result)
        : "0"(number), "D"(first), "S"(second), "d"(third),
          "r"(fourth_register), "r"(fifth_register), "r"(sixth_register)
        : "rcx", "r11", "memory");
    return result;
}

static long fail(int error)
{
    errno = error;
    return -1;
}

static long result(long value)
{
    if (value >= 0)
        return value;
    return fail(value < -INT_MAX ? EIO : (int)-value);
}

static long socket_op(int fd, unsigned operation, const void *input, size_t input_size,
                      void *output, size_t output_size)
{
    long value = native_call(SYS_NET_SOCKET_OP, fd, operation, (uintptr_t)input,
                             input_size, (uintptr_t)output, output_size);
    /* The native operation returns EBADF for an FD of an unrelated type. */
    if (value == -EBADF && fcntl(fd, F_GETFD) >= 0)
        return -ENOTSOCK;
    return value;
}

static long query_endpoints(int fd, struct dunit_socket_endpoints6 *endpoints)
{
    long value = socket_op(fd, DUNIT_SOCKET_OP_ENDPOINTS, NULL, 0,
                           endpoints, sizeof *endpoints);
    if (value >= 0 && value != sizeof *endpoints)
        return -EIO;
    return value;
}

static struct sock_state get_state(int fd)
{
    struct sock_state copy = {
        .active = 0,
        .nonblocking = 1,
        .domain = AF_INET,
        .sock_type = 0,
        .protocol = 0,
        .rcv_timeout_ms = 0,
        .snd_timeout_ms = 0,
    };
    if (fd < 0 || fd >= MAX_TRACKED_FDS)
        return copy;
    lock_states();
    if (fd_states[fd].active)
        copy = fd_states[fd];
    unlock_states();
    return copy;
}

static void set_state(int fd, int nonblocking, int domain, int sock_type, int protocol)
{
    if (fd < 0 || fd >= MAX_TRACKED_FDS)
        return;
    lock_states();
    fd_states[fd].active = 1;
    fd_states[fd].nonblocking = nonblocking ? 1 : 0;
    fd_states[fd].domain = domain;
    fd_states[fd].sock_type = sock_type;
    fd_states[fd].protocol = protocol;
    fd_states[fd].rcv_timeout_ms = 0;
    fd_states[fd].snd_timeout_ms = 0;
    unlock_states();
}

static void clear_state(int fd)
{
    if (fd < 0 || fd >= MAX_TRACKED_FDS)
        return;
    lock_states();
    memset(&fd_states[fd], 0, sizeof fd_states[fd]);
    unlock_states();
}

static uint64_t monotonic_ms_now(void)
{
    uint64_t t[2] = {0, 0};
    if (native_call(SYS_CLOCK_GET_TIME, 1, (uintptr_t)t, 0, 0, 0, 0) < 0)
        return 0;
    return t[0] * 1000ULL + t[1] / 1000000ULL;
}

static int poll_wait(int fd, short events, uint64_t timeout_ms, uint64_t *deadline_out)
{
    int wait_ms = -1;
    if (timeout_ms > 0) {
        uint64_t now = monotonic_ms_now();
        if (*deadline_out == 0) {
            uint64_t cap = UINT64_MAX - now;
            *deadline_out = now + (timeout_ms > cap ? cap : timeout_ms);
        }
        if (now >= *deadline_out)
            return 0;
        uint64_t rem = *deadline_out - now;
        wait_ms = rem > (uint64_t)INT_MAX ? INT_MAX : (int)rem;
    }
    struct pollfd pfd = { .fd = fd, .events = events };
    int ready = poll(&pfd, 1, wait_ms);
    if (ready < 0)
        return -1;
    if (ready == 0)
        return 0;
    if (pfd.revents & POLLNVAL) {
        errno = EBADF;
        return -1;
    }
    return 1;
}

static int endpoint_error(const struct sockaddr *address, socklen_t length,
                          struct dunit_socket_endpoint6 *endpoint)
{
    if (!address)
        return EFAULT;
    if (length < sizeof(sa_family_t))
        return EINVAL;
    memset(endpoint, 0, sizeof *endpoint);
    sa_family_t fam = 0;
    memcpy(&fam, address, sizeof fam);
    if (fam == AF_INET) {
        if (length < sizeof(struct sockaddr_in))
            return EINVAL;
        struct sockaddr_in ipv4;
        memcpy(&ipv4, address, sizeof ipv4);
        endpoint->family = DUNIT_SOCKET_FAMILY_IPV4;
        endpoint->port = ntohs(ipv4.sin_port);
        memcpy(endpoint->addr, &ipv4.sin_addr.s_addr, 4);
        return 0;
    }
    if (fam == AF_INET6) {
        if (length < sizeof(struct sockaddr_in6))
            return EINVAL;
        struct sockaddr_in6 ipv6;
        memcpy(&ipv6, address, sizeof ipv6);
        endpoint->family = DUNIT_SOCKET_FAMILY_IPV6;
        endpoint->port = ntohs(ipv6.sin6_port);
        endpoint->scope_id = ipv6.sin6_scope_id;
        memcpy(endpoint->addr, &ipv6.sin6_addr.s6_addr, 16);
        return 0;
    }
    return EAFNOSUPPORT;
}

static void put_address(const struct dunit_socket_endpoint6 *endpoint,
                        struct sockaddr *address, socklen_t *length)
{
    if (endpoint->family == DUNIT_SOCKET_FAMILY_IPV6) {
        struct sockaddr_in6 ipv6 = {0};
        ipv6.sin6_family = AF_INET6;
        ipv6.sin6_port = htons(endpoint->port);
        ipv6.sin6_scope_id = endpoint->scope_id;
        memcpy(&ipv6.sin6_addr.s6_addr, endpoint->addr, 16);
        size_t copied = *length < sizeof ipv6 ? *length : sizeof ipv6;
        memcpy(address, &ipv6, copied);
        *length = sizeof ipv6;
        return;
    }
    struct sockaddr_in ipv4 = {0};
    ipv4.sin_family = AF_INET;
    ipv4.sin_port = htons(endpoint->port);
    memcpy(&ipv4.sin_addr.s_addr, endpoint->addr, 4);
    size_t copied = *length < sizeof ipv4 ? *length : sizeof ipv4;
    memcpy(address, &ipv4, copied);
    *length = sizeof ipv4;
}

int dunit_socket_use_grant(uint32_t grant)
{
    if (grant != 0) {
        long rights = native_call(SYS_HANDLE_RIGHTS, grant, 0, 0, 0, 0, 0);
        if (rights < 0)
            return (int)result(rights);
        if (!(rights & DUNIT_RIGHT_NET_CONNECT))
            return (int)fail(EACCES);
    }
    __atomic_store_n(&selected_grant, grant, __ATOMIC_RELEASE);
    return 0;
}

int dunit_socket_set_nonblocking(int fd, int nonblocking)
{
    struct dunit_socket_endpoints6 endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return (int)result(value);
    if (fd < 0 || fd >= MAX_TRACKED_FDS)
        return (int)fail(EBADF);
    int dom = endpoints.local.family == DUNIT_SOCKET_FAMILY_IPV6 ? AF_INET6 : AF_INET;
    lock_states();
    fd_states[fd].active = 1;
    fd_states[fd].domain = dom;
    fd_states[fd].nonblocking = nonblocking ? 1 : 0;
    unlock_states();
    return 0;
}

static int await_open(int fd)
{
    struct pollfd interest = { .fd = fd, .events = POLLOUT };
    int ready = poll(&interest, 1, OPEN_TIMEOUT_MS);
    if (ready < 0)
        return -1;
    if (!ready)
        return (int)fail(ETIMEDOUT);
    if (interest.revents & (POLLERR | POLLHUP | POLLNVAL)) {
        int32_t error = 0;
        long value = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &error, sizeof error);
        if (value < 0)
            return (int)result(value);
        if (value != sizeof error || error >= 0)
            return (int)fail(EIO);
        return (int)result(error);
    }
    if (!(interest.revents & POLLOUT))
        return (int)fail(EIO);
    return 0;
}

int socket(int domain, int type, int protocol)
{
    if (domain != AF_INET && domain != AF_INET6)
        return (int)fail(EAFNOSUPPORT);
    int base_type = type & ~(SOCK_NONBLOCK | SOCK_CLOEXEC);
    unsigned native_protocol;
    int resolved_proto;
    switch (base_type) {
    case SOCK_STREAM:
        if (protocol && protocol != IPPROTO_TCP)
            return (int)fail(EPROTONOSUPPORT);
        native_protocol = DUNIT_SOCKET_PROTO_TCP;
        resolved_proto = IPPROTO_TCP;
        break;
    case SOCK_DGRAM:
        if (protocol && protocol != IPPROTO_UDP)
            return (int)fail(EPROTONOSUPPORT);
        native_protocol = DUNIT_SOCKET_PROTO_UDP;
        resolved_proto = IPPROTO_UDP;
        break;
    default:
        return (int)fail(EPROTONOSUPPORT);
    }
    uint32_t grant = __atomic_load_n(&selected_grant, __ATOMIC_ACQUIRE);
    if (!grant)
        return (int)fail(EACCES);
    const struct dunit_socket_options options = { .version = DUNIT_SOCKET_VERSION_V2 };
    long value = native_call(SYS_NET_SOCKET_OPEN, grant, native_protocol, (uintptr_t)&options,
                             sizeof options, 0, 0);
    if (value < 0)
        return (int)result(value);
    int fd = (int)value;
    if (await_open(fd) == 0 &&
        ((type & SOCK_CLOEXEC) || fcntl(fd, F_SETFD, 0) == 0)) {
        set_state(fd, (type & SOCK_NONBLOCK) != 0, domain, base_type, resolved_proto);
        return fd;
    }
    int error = errno;
    clear_state(fd);
    close(fd);
    return (int)fail(error);
}

static int await_write_op(int fd, unsigned op, const void *input, size_t input_size)
{
    uint64_t deadline = 0;
    for (;;) {
        long value = socket_op(fd, op, input, input_size, NULL, 0);
        if (value == 0)
            return 0;
        if (value != -EAGAIN)
            return (int)value;
        int waited = poll_wait(fd, POLLOUT, OPEN_TIMEOUT_MS, &deadline);
        if (waited < 0)
            return -errno;
        if (waited == 0)
            return -ETIMEDOUT;
        int32_t sock_err = 0;
        long err_res = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &sock_err, sizeof sock_err);
        if (err_res < 0)
            return (int)err_res;
        if (err_res != sizeof sock_err || sock_err > 0)
            return -EIO;
        if (sock_err < 0)
            return (int)sock_err;
    }
}

int bind(int fd, const struct sockaddr *address, socklen_t length)
{
    struct dunit_socket_endpoint6 endpoint;
    int error = endpoint_error(address, length, &endpoint);
    if (error)
        return (int)fail(error);
    int res = await_write_op(fd, DUNIT_SOCKET_OP_BIND, &endpoint, sizeof endpoint);
    if (res == -EBUSY)
        return (int)fail(EADDRINUSE);
    return res < 0 ? (int)result(res) : 0;
}

int listen(int fd, int backlog)
{
    uint32_t bl = backlog <= 0 ? 1u : (uint32_t)backlog;
    int res = await_write_op(fd, DUNIT_SOCKET_OP_LISTEN, &bl, sizeof bl);
    if (res == -EBUSY)
        return (int)fail(EADDRINUSE);
    return res < 0 ? (int)result(res) : 0;
}

int accept4(int fd, struct sockaddr *address, socklen_t *length, int flags)
{
    if (flags & ~(SOCK_NONBLOCK | SOCK_CLOEXEC))
        return (int)fail(EINVAL);
    if (address && !length)
        return (int)fail(EFAULT);

    struct sock_state st = get_state(fd);
    struct pollfd pre = { .fd = fd, .events = POLLIN };
    int pre_ready = poll(&pre, 1, 0) > 0 && (pre.revents & POLLIN);
    uint64_t rcv_deadline = 0;
    uint64_t op_deadline = 0;

    for (;;) {
        struct dunit_socket_endpoint6 remote = {0};
        long res = socket_op(fd, DUNIT_SOCKET_OP_ACCEPT, NULL, 0,
                             address ? &remote : NULL, address ? sizeof remote : 0);
        if (res >= 0) {
            int new_fd = (int)res;
            if (!(flags & SOCK_CLOEXEC) && fcntl(new_fd, F_SETFD, 0) != 0) {
                int err = errno;
                close(new_fd);
                return (int)fail(err);
            }
            int child_dom = remote.family == DUNIT_SOCKET_FAMILY_IPV6 ? AF_INET6 : st.domain;
            set_state(new_fd, (flags & SOCK_NONBLOCK) != 0, child_dom, SOCK_STREAM, IPPROTO_TCP);
            if (address)
                put_address(&remote, address, length);
            return new_fd;
        }
        if (res != -EAGAIN)
            return (int)result(res);
        if (pre_ready) {
            int waited = poll_wait(fd, POLLIN, OPEN_TIMEOUT_MS, &op_deadline);
            if (waited <= 0)
                return waited == 0 ? (int)fail(EAGAIN) : -1;
        } else {
            if (st.nonblocking)
                return (int)fail(EAGAIN);
            int waited = poll_wait(fd, POLLIN, st.rcv_timeout_ms, &rcv_deadline);
            if (waited <= 0)
                return waited == 0 ? (int)fail(EAGAIN) : -1;
            pre_ready = 1;
            op_deadline = 0;
        }
        int32_t sock_err = 0;
        long err_res = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &sock_err, sizeof sock_err);
        if (err_res == sizeof sock_err && sock_err < 0)
            return (int)result(sock_err);
    }
}

int accept(int fd, struct sockaddr *address, socklen_t *length)
{
    return accept4(fd, address, length, 0);
}

int connect(int fd, const struct sockaddr *address, socklen_t length)
{
    struct dunit_socket_endpoint6 endpoint;
    int error = endpoint_error(address, length, &endpoint);
    if (error)
        return (int)fail(error);
    struct sock_state st = get_state(fd);
    long value = socket_op(fd, DUNIT_SOCKET_OP_CONNECT, &endpoint, sizeof endpoint, NULL, 0);
    if (value != -EAGAIN)
        return (int)result(value);
    if (st.nonblocking)
        return (int)fail(EINPROGRESS);

    uint64_t deadline = 0;
    for (;;) {
        int waited = poll_wait(fd, POLLOUT, st.snd_timeout_ms, &deadline);
        if (waited < 0)
            return -1;
        if (waited == 0)
            return (int)fail(ETIMEDOUT);
        int32_t sock_err = 0;
        long err_res = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &sock_err, sizeof sock_err);
        if (err_res < 0)
            return (int)result(err_res);
        if (err_res != sizeof sock_err || sock_err > 0)
            return (int)fail(EIO);
        if (sock_err < 0)
            return (int)result(sock_err);
        struct dunit_socket_endpoints6 eps;
        long ep_res = query_endpoints(fd, &eps);
        if (ep_res < 0)
            return (int)result(ep_res);
        if (eps.remote.port != 0)
            return 0;
    }
}

ssize_t send(int fd, const void *buffer, size_t length, int flags)
{
    if (flags & ~(MSG_DONTWAIT | MSG_NOSIGNAL))
        return fail(EOPNOTSUPP);
    if (length && !buffer)
        return fail(EFAULT);
    struct dunit_socket_endpoints6 endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return result(value);
    if (!endpoints.remote.port)
        return fail(ENOTCONN);
    if (!length) {
        value = socket_op(fd, DUNIT_SOCKET_OP_SEND_TO, &endpoints.remote,
                          sizeof endpoints.remote, NULL, 0);
        return result(value == -EOPNOTSUPP ? 0 : value);
    }

    struct sock_state st = get_state(fd);
    if (st.nonblocking || (flags & MSG_DONTWAIT))
        return result(native_call(SYS_WRITE, fd, (uintptr_t)buffer, length, 0, 0, 0));

    size_t sent = 0;
    uint64_t deadline = 0;
    const unsigned char *src = (const unsigned char *)buffer;
    while (sent < length) {
        value = native_call(SYS_WRITE, fd, (uintptr_t)(src + sent), length - sent, 0, 0, 0);
        if (value > 0) {
            sent += (size_t)value;
            if (st.sock_type == SOCK_DGRAM)
                break;
            continue;
        }
        if (value == -EAGAIN) {
            int waited = poll_wait(fd, POLLOUT, st.snd_timeout_ms, &deadline);
            if (waited > 0)
                continue;
            if (sent > 0)
                return (ssize_t)sent;
            return waited == 0 ? fail(EAGAIN) : -1;
        }
        if (sent > 0)
            return (ssize_t)sent;
        return result(value);
    }
    return (ssize_t)sent;
}

ssize_t sendto(int fd, const void *buffer, size_t length, int flags,
               const struct sockaddr *address, socklen_t address_length)
{
    if (!address && !address_length)
        return send(fd, buffer, length, flags);
    if (flags & ~(MSG_DONTWAIT | MSG_NOSIGNAL))
        return fail(EOPNOTSUPP);
    if (length > DUNIT_SOCKET_MAX_PAYLOAD)
        return fail(EMSGSIZE);
    if (length && !buffer)
        return fail(EFAULT);
    struct {
        struct dunit_socket_endpoint6 endpoint;
        unsigned char payload[DUNIT_SOCKET_MAX_PAYLOAD];
    } record;
    int error = endpoint_error(address, address_length, &record.endpoint);
    if (error)
        return fail(error);
    if (length)
        memcpy(record.payload, buffer, length);

    struct sock_state st = get_state(fd);
    int nonblocking = st.nonblocking || (flags & MSG_DONTWAIT);
    uint64_t deadline = 0;
    for (;;) {
        long value = socket_op(fd, DUNIT_SOCKET_OP_SEND_TO, &record,
                               sizeof record.endpoint + length, NULL, 0);
        if (value != -EAGAIN || nonblocking)
            return result(value);
        int waited = poll_wait(fd, POLLOUT, st.snd_timeout_ms, &deadline);
        if (waited < 0)
            return -1;
        if (waited == 0)
            return fail(EAGAIN);
    }
}

ssize_t recvfrom(int fd, void *buffer, size_t length, int flags,
                 struct sockaddr *address, socklen_t *address_length)
{
    if (flags & ~(MSG_DONTWAIT | MSG_TRUNC))
        return fail(EOPNOTSUPP);
    if ((length && !buffer) || (address && !address_length))
        return fail(EFAULT);
    size_t capacity = length < DUNIT_SOCKET_MAX_PAYLOAD ? length : DUNIT_SOCKET_MAX_PAYLOAD;
    struct {
        struct dunit_socket_receive6 header;
        unsigned char payload[DUNIT_SOCKET_MAX_PAYLOAD];
    } record;

    struct sock_state st = get_state(fd);
    int nonblocking = st.nonblocking || (flags & MSG_DONTWAIT);
    uint64_t deadline = 0;

    for (;;) {
        long value = socket_op(fd, DUNIT_SOCKET_OP_RECV_FROM, NULL, 0, &record,
                               sizeof record.header + capacity);
        if (value == -EOPNOTSUPP) {
            if (flags & MSG_TRUNC)
                return fail(EOPNOTSUPP);
            struct dunit_socket_endpoints6 endpoints;
            if (address) {
                value = query_endpoints(fd, &endpoints);
                if (value < 0)
                    return result(value);
                if (!endpoints.remote.port)
                    return fail(ENOTCONN);
            }
            for (;;) {
                value = native_call(SYS_READ, fd, (uintptr_t)buffer, capacity, 0, 0, 0);
                if (value != -EAGAIN || nonblocking) {
                    if (value >= 0 && address)
                        put_address(&endpoints.remote, address, address_length);
                    return result(value);
                }
                int waited = poll_wait(fd, POLLIN, st.rcv_timeout_ms, &deadline);
                if (waited < 0)
                    return -1;
                if (waited == 0)
                    return fail(EAGAIN);
            }
        }
        if (value == -EAGAIN && !nonblocking) {
            int waited = poll_wait(fd, POLLIN, st.rcv_timeout_ms, &deadline);
            if (waited < 0)
                return -1;
            if (waited == 0)
                return fail(EAGAIN);
            continue;
        }
        if (value < 0)
            return result(value);
        if ((size_t)value < sizeof record.header ||
            (size_t)value > sizeof record.header + capacity)
            return fail(EIO);
        size_t copied = (size_t)value - sizeof record.header;
        if (copied > record.header.full_len || record.header.full_len > DUNIT_SOCKET_MAX_PAYLOAD)
            return fail(EIO);
        if (copied)
            memcpy(buffer, record.payload, copied);
        if (address)
            put_address(&record.header.source, address, address_length);
        return flags & MSG_TRUNC ? (ssize_t)record.header.full_len : (ssize_t)copied;
    }
}

ssize_t recv(int fd, void *buffer, size_t length, int flags)
{
    return recvfrom(fd, buffer, length, flags, NULL, NULL);
}

int shutdown(int fd, int how)
{
    if (how != SHUT_WR) {
        if (how != SHUT_RD && how != SHUT_RDWR)
            return (int)fail(EINVAL);
        return (int)fail(EOPNOTSUPP);
    }
    struct sock_state st = get_state(fd);
    uint64_t deadline = 0;
    for (;;) {
        long value = socket_op(fd, DUNIT_SOCKET_OP_SHUTDOWN_WRITE, NULL, 0, NULL, 0);
        if (value != -EAGAIN || st.nonblocking)
            return (int)result(value);
        int waited = poll_wait(fd, POLLOUT, st.snd_timeout_ms, &deadline);
        if (waited < 0)
            return -1;
        if (waited == 0)
            return (int)fail(ETIMEDOUT);
    }
}

static int socket_name(int fd, struct sockaddr *address, socklen_t *length, int remote)
{
    if (!address || !length)
        return (int)fail(EFAULT);
    struct dunit_socket_endpoints6 endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return (int)result(value);
    if (remote && !endpoints.remote.port)
        return (int)fail(ENOTCONN);
    put_address(remote ? &endpoints.remote : &endpoints.local, address, length);
    return 0;
}

int getsockname(int fd, struct sockaddr *address, socklen_t *length)
{
    return socket_name(fd, address, length, 0);
}

int getpeername(int fd, struct sockaddr *address, socklen_t *length)
{
    return socket_name(fd, address, length, 1);
}

static int parse_timeval_ms(const void *data, socklen_t length, uint64_t *out_ms)
{
    if (!data)
        return EFAULT;
    if (length < sizeof(struct timeval))
        return EINVAL;
    struct timeval tv;
    memcpy(&tv, data, sizeof tv);
    if (tv.tv_sec < 0 || tv.tv_usec < 0 || tv.tv_usec >= 1000000)
        return EDOM;
    if (tv.tv_sec == 0 && tv.tv_usec == 0) {
        *out_ms = 0;
        return 0;
    }
    uint64_t sec_ms = (uint64_t)tv.tv_sec > (UINT64_MAX - 1000ULL) / 1000ULL
                          ? UINT64_MAX - 1000ULL
                          : (uint64_t)tv.tv_sec * 1000ULL;
    uint64_t usec_ms = ((uint64_t)tv.tv_usec + 999ULL) / 1000ULL;
    *out_ms = sec_ms + usec_ms;
    return 0;
}

static int write_timeval_ms(uint64_t ms, void *data, socklen_t *length)
{
    if (!data || !length)
        return EFAULT;
    if (*length < sizeof(struct timeval))
        return EINVAL;
    struct timeval tv = {
        .tv_sec = (time_t)(ms / 1000ULL),
        .tv_usec = (suseconds_t)((ms % 1000ULL) * 1000ULL),
    };
    memcpy(data, &tv, sizeof tv);
    *length = sizeof tv;
    return 0;
}

static int write_int_opt(int val, void *data, socklen_t *length)
{
    if (!data || !length)
        return EFAULT;
    if (*length < sizeof(int))
        return EINVAL;
    memcpy(data, &val, sizeof val);
    *length = sizeof val;
    return 0;
}

int getsockopt(int fd, int level, int option, void *data, socklen_t *length)
{
    struct dunit_socket_endpoints6 endpoints;
    long ep_res = query_endpoints(fd, &endpoints);
    if (ep_res < 0)
        return (int)result(ep_res);
    struct sock_state st = get_state(fd);
    if (level == IPPROTO_TCP && option == TCP_NODELAY) {
        if (st.active && st.protocol != IPPROTO_TCP)
            return (int)fail(ENOPROTOOPT);
        int err = write_int_opt(1, data, length);
        return err ? (int)fail(err) : 0;
    }
    if (level != SOL_SOCKET)
        return (int)fail(ENOPROTOOPT);
    if (option == SO_ERROR) {
        if (!data || !length)
            return (int)fail(EFAULT);
        if (*length < sizeof(int))
            return (int)fail(EINVAL);
        int32_t native_error = 0;
        long value = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &native_error, sizeof native_error);
        if (value < 0)
            return (int)result(value);
        if (value != sizeof native_error || native_error > 0 || native_error == INT32_MIN)
            return (int)fail(EIO);
        int error = -native_error;
        memcpy(data, &error, sizeof error);
        *length = sizeof error;
        return 0;
    }

    if (option == SO_RCVTIMEO || option == SO_SNDTIMEO) {
        uint64_t ms = option == SO_RCVTIMEO ? st.rcv_timeout_ms : st.snd_timeout_ms;
        int err = write_timeval_ms(ms, data, length);
        return err ? (int)fail(err) : 0;
    }
    if (option == SO_DOMAIN) {
        int dom = endpoints.local.family == DUNIT_SOCKET_FAMILY_IPV6 ? AF_INET6
                : st.active ? st.domain : AF_INET;
        int err = write_int_opt(dom, data, length);
        return err ? (int)fail(err) : 0;
    }
    if (option == SO_TYPE && st.active && st.sock_type) {
        int err = write_int_opt(st.sock_type, data, length);
        return err ? (int)fail(err) : 0;
    }
    if (option == SO_PROTOCOL && st.active && st.protocol) {
        int err = write_int_opt(st.protocol, data, length);
        return err ? (int)fail(err) : 0;
    }
    return (int)fail(ENOPROTOOPT);
}

int setsockopt(int fd, int level, int option, const void *data, socklen_t length)
{
    struct dunit_socket_endpoints6 endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return (int)result(value);
    if (level == IPPROTO_TCP && option == TCP_NODELAY) {
        struct sock_state st = get_state(fd);
        if (st.active && st.protocol != IPPROTO_TCP)
            return (int)fail(ENOPROTOOPT);
        if (!data)
            return (int)fail(EFAULT);
        if (length < sizeof(int))
            return (int)fail(EINVAL);
        int enabled = 0;
        memcpy(&enabled, data, sizeof enabled);
        return enabled ? 0 : (int)fail(ENOPROTOOPT);
    }
    if (level == IPPROTO_IPV6 && option == IPV6_V6ONLY) {
        if (!data)
            return (int)fail(EFAULT);
        if (length < sizeof(int))
            return (int)fail(EINVAL);
        return 0;
    }
    if (level != SOL_SOCKET || (option != SO_RCVTIMEO && option != SO_SNDTIMEO))
        return (int)fail(ENOPROTOOPT);
    uint64_t ms = 0;
    int err = parse_timeval_ms(data, length, &ms);
    if (err)
        return (int)fail(err);
    if (fd < 0 || fd >= MAX_TRACKED_FDS)
        return (int)fail(EBADF);
    lock_states();
    if (!fd_states[fd].active) {
        fd_states[fd].active = 1;
        fd_states[fd].domain = endpoints.local.family == DUNIT_SOCKET_FAMILY_IPV6 ? AF_INET6 : AF_INET;
        fd_states[fd].nonblocking = 1;
    }
    if (option == SO_RCVTIMEO)
        fd_states[fd].rcv_timeout_ms = ms;
    else
        fd_states[fd].snd_timeout_ms = ms;
    unlock_states();
    return 0;
}

static int parse_service_port(const char *service, int flags, uint16_t *port_out)
{
    if (!service || !*service) {
        *port_out = 0;
        return 0;
    }
    unsigned long val = 0;
    const char *p = service;
    int numeric = 1;
    while (*p) {
        if (*p < '0' || *p > '9') {
            numeric = 0;
            break;
        }
        val = val * 10 + (unsigned long)(*p - '0');
        if (val > 65535)
            return EAI_SERVICE;
        p++;
    }
    if (numeric) {
        *port_out = (uint16_t)val;
        return 0;
    }
    if (flags & AI_NUMERICSERV)
        return EAI_NONAME;
    if (strcmp(service, "http") == 0)
        *port_out = 80;
    else if (strcmp(service, "https") == 0)
        *port_out = 443;
    else if (strcmp(service, "domain") == 0)
        *port_out = 53;
    else
        return EAI_SERVICE;
    return 0;
}

struct resolved_addr {
    int family;
    uint32_t scope_id;
    uint8_t addr[16];
};

static int resolve_dns_dual(const char *name, struct resolved_addr *out, size_t max_out, size_t *out_count)
{
    size_t len = strlen(name);
    if (len == 0 || len > DUNIT_SOCKET_MAX_DNS_NAME)
        return EAI_NONAME;
    uint32_t grant = __atomic_load_n(&selected_grant, __ATOMIC_ACQUIRE);
    if (!grant)
        return EAI_FAIL;
    const struct dunit_socket_options options = { .version = DUNIT_SOCKET_VERSION_V2 };
    long fd_res = native_call(SYS_NET_SOCKET_OPEN, grant, DUNIT_SOCKET_PROTO_DNS,
                              (uintptr_t)&options, sizeof options, 0, 0);
    if (fd_res < 0)
        return fd_res == -EAGAIN ? EAI_AGAIN : EAI_FAIL;
    int fd = (int)fd_res;
    if (await_open(fd) != 0) {
        int err = errno;
        close(fd);
        return err == ETIMEDOUT || err == EAGAIN ? EAI_AGAIN : EAI_FAIL;
    }

    uint64_t deadline = 0;
    size_t written = 0;
    while (written < len) {
        long n = native_call(SYS_WRITE, fd, (uintptr_t)(name + written), len - written, 0, 0, 0);
        if (n > 0) {
            written += (size_t)n;
            continue;
        }
        if (n == -EAGAIN) {
            int waited = poll_wait(fd, POLLOUT, DNS_TIMEOUT_MS, &deadline);
            if (waited > 0)
                continue;
            close(fd);
            return waited == 0 ? EAI_AGAIN : EAI_FAIL;
        }
        close(fd);
        return n == -EINVAL ? EAI_NONAME : EAI_FAIL;
    }

    uint8_t buf[MAX_DNS_WIRE_LEN];
    for (;;) {
        long n = native_call(SYS_READ, fd, (uintptr_t)buf, sizeof buf, 0, 0, 0);
        if (n == 4) {
            close(fd);
            out[0].family = AF_INET;
            out[0].scope_id = 0;
            memset(out[0].addr, 0, 16);
            memcpy(out[0].addr, buf, 4);
            *out_count = 1;
            return 0;
        }
        if (n > 2) {
            close(fd);
            uint8_t cnt = buf[0];
            size_t expected = 2 + (size_t)cnt * 22;
            if (cnt == 0 || cnt > 16 || buf[1] != 0 || (size_t)n != expected)
                return EAI_FAIL;
            size_t total = (size_t)cnt < max_out ? (size_t)cnt : max_out;
            for (size_t i = 0; i < total; i++) {
                const uint8_t *rec = buf + 2 + i * 22;
                out[i].family = rec[0] == DUNIT_SOCKET_FAMILY_IPV6 ? AF_INET6 : AF_INET;
                uint32_t sc = 0;
                memcpy(&sc, rec + 2, 4);
                out[i].scope_id = sc;
                memcpy(out[i].addr, rec + 6, 16);
            }
            *out_count = total;
            return 0;
        }
        if (n >= 0) {
            close(fd);
            return EAI_FAIL;
        }
        if (n != -EAGAIN) {
            close(fd);
            if (n == -ENOENT || n == -EINVAL)
                return EAI_NONAME;
            return EAI_FAIL;
        }
        int waited = poll_wait(fd, POLLIN, DNS_TIMEOUT_MS, &deadline);
        if (waited <= 0) {
            close(fd);
            return waited == 0 ? EAI_AGAIN : EAI_FAIL;
        }
        int32_t sock_err = 0;
        long err_res = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &sock_err, sizeof sock_err);
        if (err_res == sizeof sock_err && sock_err < 0) {
            close(fd);
            if (sock_err == -ENOENT || sock_err == -EINVAL)
                return EAI_NONAME;
            if (sock_err == -EAGAIN)
                return EAI_AGAIN;
            return EAI_FAIL;
        }
    }
}

static struct addrinfo *alloc_ai_node(const struct resolved_addr *addr, uint16_t port,
                                      int socktype, int protocol, const char *canon)
{
    struct addrinfo *ai = calloc(1, sizeof *ai);
    char *cname = canon ? strdup(canon) : NULL;
    if (!ai || (canon && !cname)) {
        free(cname);
        free(ai);
        return NULL;
    }
    if (addr->family == AF_INET6) {
        struct sockaddr_in6 *sa6 = calloc(1, sizeof *sa6);
        if (!sa6) {
            free(cname);
            free(ai);
            return NULL;
        }
        sa6->sin6_family = AF_INET6;
        sa6->sin6_port = htons(port);
        sa6->sin6_scope_id = addr->scope_id;
        memcpy(&sa6->sin6_addr.s6_addr, addr->addr, 16);
        ai->ai_family = AF_INET6;
        ai->ai_addrlen = sizeof *sa6;
        ai->ai_addr = (struct sockaddr *)sa6;
    } else {
        struct sockaddr_in *sa4 = calloc(1, sizeof *sa4);
        if (!sa4) {
            free(cname);
            free(ai);
            return NULL;
        }
        sa4->sin_family = AF_INET;
        sa4->sin_port = htons(port);
        memcpy(&sa4->sin_addr.s_addr, addr->addr, 4);
        ai->ai_family = AF_INET;
        ai->ai_addrlen = sizeof *sa4;
        ai->ai_addr = (struct sockaddr *)sa4;
    }
    ai->ai_socktype = socktype;
    ai->ai_protocol = protocol;
    ai->ai_canonname = cname;
    return ai;
}

int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints,
                struct addrinfo **res)
{
    if (!res)
        return EAI_NONAME;
    *res = NULL;
    if (!node && !service)
        return EAI_NONAME;

    int flags = hints ? hints->ai_flags : 0;
    int family = hints ? hints->ai_family : AF_UNSPEC;
    int socktype = hints ? hints->ai_socktype : 0;
    int protocol = hints ? hints->ai_protocol : 0;

    int allowed_flags = AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST | AI_V4MAPPED | AI_ALL |
                        AI_ADDRCONFIG | AI_NUMERICSERV;
    if (flags & ~allowed_flags)
        return EAI_BADFLAGS;
    if (family != AF_UNSPEC && family != AF_INET && family != AF_INET6)
        return EAI_FAMILY;

    int emit_stream = 0;
    int emit_dgram = 0;
    if (socktype == 0 && protocol == 0) {
        emit_stream = 1;
        emit_dgram = 1;
    } else if ((socktype == 0 || socktype == SOCK_STREAM) &&
               (protocol == 0 || protocol == IPPROTO_TCP)) {
        emit_stream = 1;
    } else if ((socktype == 0 || socktype == SOCK_DGRAM) &&
               (protocol == 0 || protocol == IPPROTO_UDP)) {
        emit_dgram = 1;
    } else {
        return EAI_SOCKTYPE;
    }

    uint16_t port = 0;
    int srv_err = parse_service_port(service, flags, &port);
    if (srv_err)
        return srv_err;

    struct resolved_addr addrs[16];
    size_t addr_count = 0;
    const char *canon = NULL;

    if (!node) {
        int out_fam = family == AF_INET6 ? AF_INET6 : AF_INET;
        addrs[0].family = out_fam;
        addrs[0].scope_id = 0;
        memset(addrs[0].addr, 0, 16);
        if (!(flags & AI_PASSIVE)) {
            if (out_fam == AF_INET6) {
                addrs[0].addr[15] = 1;
            } else {
                addrs[0].addr[0] = 127;
                addrs[0].addr[3] = 1;
            }
        }
        addr_count = 1;
    } else {
        struct in_addr addr4;
        struct in6_addr addr6;
        if (family != AF_INET6 && inet_pton(AF_INET, node, &addr4) == 1) {
            addrs[0].family = AF_INET;
            addrs[0].scope_id = 0;
            memset(addrs[0].addr, 0, 16);
            memcpy(addrs[0].addr, &addr4.s_addr, 4);
            addr_count = 1;
            if (flags & AI_CANONNAME)
                canon = node;
        } else if (family != AF_INET && inet_pton(AF_INET6, node, &addr6) == 1) {
            addrs[0].family = AF_INET6;
            addrs[0].scope_id = 0;
            memcpy(addrs[0].addr, &addr6.s6_addr, 16);
            addr_count = 1;
            if (flags & AI_CANONNAME)
                canon = node;
        } else if (flags & AI_NUMERICHOST) {
            return EAI_NONAME;
        } else {
            struct resolved_addr raw[16];
            size_t raw_count = 0;
            int dns_err = resolve_dns_dual(node, raw, 16, &raw_count);
            if (dns_err)
                return dns_err;
            for (size_t i = 0; i < raw_count; i++) {
                if (family == AF_UNSPEC || family == raw[i].family)
                    addrs[addr_count++] = raw[i];
            }
            if (addr_count == 0)
                return EAI_NODATA;
            if (flags & AI_CANONNAME)
                canon = node;
        }
    }

    struct addrinfo *head = NULL;
    struct addrinfo *tail = NULL;
    for (size_t i = 0; i < addr_count; i++) {
        if (emit_stream) {
            struct addrinfo *n = alloc_ai_node(&addrs[i], port, SOCK_STREAM, IPPROTO_TCP, canon);
            if (!n) {
                freeaddrinfo(head);
                return EAI_MEMORY;
            }
            canon = NULL;
            if (!head)
                head = n;
            else
                tail->ai_next = n;
            tail = n;
        }
        if (emit_dgram) {
            struct addrinfo *n = alloc_ai_node(&addrs[i], port, SOCK_DGRAM, IPPROTO_UDP, canon);
            if (!n) {
                freeaddrinfo(head);
                return EAI_MEMORY;
            }
            canon = NULL;
            if (!head)
                head = n;
            else
                tail->ai_next = n;
            tail = n;
        }
    }
    *res = head;
    return 0;
}

void freeaddrinfo(struct addrinfo *res)
{
    while (res) {
        struct addrinfo *next = res->ai_next;
        free(res->ai_addr);
        free(res->ai_canonname);
        free(res);
        res = next;
    }
}

const char *gai_strerror(int ecode)
{
    switch (ecode) {
    case 0:
        return "Success";
    case EAI_BADFLAGS:
        return "Invalid value for ai_flags";
    case EAI_NONAME:
        return "Name or service not known";
    case EAI_AGAIN:
        return "Temporary failure in name resolution";
    case EAI_FAIL:
        return "Non-recoverable failure in name resolution";
    case EAI_NODATA:
        return "No address associated with hostname";
    case EAI_FAMILY:
        return "ai_family not supported";
    case EAI_SOCKTYPE:
        return "ai_socktype not supported";
    case EAI_SERVICE:
        return "Servname not supported for ai_socktype";
    case EAI_MEMORY:
        return "Memory allocation failure";
    case EAI_SYSTEM:
        return "System error";
    case EAI_OVERFLOW:
        return "Argument buffer overflow";
    default:
        return "Unknown error";
    }
}

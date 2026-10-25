//! Host-side entry point generation; no target runtime or ABI logic here.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemFn};

fn validate(function: &ItemFn, attribute: &str) -> syn::Result<()> {
    let signature = &function.sig;
    if !signature.inputs.is_empty()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
        || signature.asyncness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.constness.is_some()
        || signature.variadic.is_some()
        || signature.ident == "_start"
        || function
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("no_mangle") || attr.path().is_ident("export_name"))
    {
        return Err(syn::Error::new_spanned(signature, format!("dunit::{attribute} requires a safe, synchronous function without arguments, generics or export attributes")));
    }
    Ok(())
}

fn expand(function: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    validate(&function, "main")?;
    let name = &function.sig.ident;
    Ok(quote! {
        #[cfg(not(test))]
        #function
        #[no_mangle]
        pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
            // The Dunit ELF loader supplies the validated process-entry stack.
            #[cfg(not(test))]
            unsafe { ::dunit::__private::start(argc, argv, envp, #name) }
            #[cfg(test)]
            unsafe { ::dunit::testing::start(argc, argv, envp, __dunit_test_main) }
        }
        #[panic_handler]
        fn __dunit_panic(info: &::core::panic::PanicInfo<'_>) -> ! {
            ::dunit::__private::panic(info)
        }
    })
}

#[proc_macro_attribute]
pub fn main(attributes: TokenStream, item: TokenStream) -> TokenStream {
    if !attributes.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "dunit::main takes no options",
        )
        .to_compile_error()
        .into();
    }
    match expand(parse_macro_input!(item as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_test(function: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    validate(&function, "test")?;
    let conditions = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"));
    let name = &function.sig.ident;
    let wrapper = format_ident!("__dunit_test_{}", name);
    Ok(quote! {
        #[cfg(test)]
        #function
        #[cfg(test)]
        #[test_case]
        #(#conditions)*
        fn #wrapper() -> ::dunit::Result<()> {
            ::dunit::testing::case(concat!(module_path!(), "::", stringify!(#name)), || {
                ::dunit::testing::Outcome::into_result(#name())
            })
        }
    })
}
#[proc_macro_attribute]
pub fn test(attributes: TokenStream, item: TokenStream) -> TokenStream {
    if !attributes.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "dunit::test takes no options",
        )
        .to_compile_error()
        .into();
    }
    match expand_test(parse_macro_input!(item as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{expand, expand_test};
    #[test]
    fn rejects_unsupported_entry_signatures() {
        for source in [
            "async fn entry() {}",
            "unsafe fn entry() {}",
            "fn entry(x: u8) {}",
            "fn entry<T>() {}",
            "extern \"C\" fn entry() {}",
            "const fn entry() {}",
            "fn _start() {}",
            "#[no_mangle] fn entry() {}",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err(), "{source}");
            assert!(
                expand_test(syn::parse_str(source).unwrap()).is_err(),
                "{source}"
            );
        }
    }
    #[test]
    fn test_wrapper_preserves_conditions_and_uses_compiler_discovery() {
        let tokens =
            expand_test(syn::parse_str("#[cfg(feature = \"optional\")] fn sample() {} ").unwrap())
                .unwrap();
        let output: syn::File = syn::parse2(tokens).unwrap();
        assert_eq!(output.items.len(), 2);
        let syn::Item::Fn(wrapper) = &output.items[1] else {
            panic!()
        };
        assert_eq!(wrapper.sig.ident, "__dunit_test_sample");
        assert!(wrapper
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("test_case")));
        assert_eq!(
            wrapper
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("cfg"))
                .count(),
            2
        );
    }
    #[test]
    fn generates_one_loader_entry_and_panic_handler() {
        let tokens =
            expand(syn::parse_str("fn app() -> dunit::Result<()> { Ok(()) }").unwrap()).unwrap();
        let output: syn::File = syn::parse2(tokens).unwrap();
        assert_eq!(output.items.len(), 3);
        let syn::Item::Fn(entry) = &output.items[1] else {
            panic!()
        };
        assert_eq!(entry.sig.ident, "_start");
        assert_eq!(entry.sig.inputs.len(), 3);
        let syn::Item::Fn(panic) = &output.items[2] else {
            panic!()
        };
        assert!(panic
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("panic_handler")));
    }
}

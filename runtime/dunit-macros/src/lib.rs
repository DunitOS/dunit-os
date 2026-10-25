//! Host-side entry point generation; no target runtime or ABI logic here.
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

fn expand(function: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
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
        return Err(syn::Error::new_spanned(signature, "dunit::main requires a safe, synchronous fn with no arguments or generics, returning dunit::Result<()>"));
    }
    let name = &signature.ident;
    Ok(quote! {
        #function
        #[no_mangle]
        pub extern "C" fn _start(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
            // The Dunit ELF loader supplies the validated process-entry stack.
            unsafe { ::dunit::__private::start(argc, argv, envp, #name) }
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

#[cfg(test)]
mod tests {
    use super::*;
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
        }
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

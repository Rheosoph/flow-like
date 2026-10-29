use crate::error::{host_compatibility_error, WasmResult};
use wasmtime::component::{Component, InstancePre, Linker};

/// Resolve imports and their signatures without allocating a guest store or running guest code.
pub(crate) fn prepare<T: 'static>(
    linker: &Linker<T>,
    component: &Component,
) -> WasmResult<InstancePre<T>> {
    linker
        .instantiate_pre(component)
        .map_err(host_compatibility_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Module, Store};

    fn component(engine: &Engine) -> Component {
        Component::new(
            engine,
            wat::parse_str(
                r#"
            (component
                (import "flow-like:node/storage-ops@0.1.0" (instance
                    (export "stat-file" (func (param "path" string) (result string)))))
                (core module $guest (func $start unreachable) (start $start))
                (core instance (instantiate $guest)))
        "#,
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn missing_method_reports_interface_and_method_before_guest_start() {
        let engine = Engine::default();
        let component = component(&engine);
        let mut linker = Linker::<()>::new(&engine);
        linker.instance("flow-like:node/storage-ops@0.1.0").unwrap();
        let error = match prepare(&linker, &component) {
            Ok(_) => panic!("missing import accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("storage-ops@0.1.0"), "{error}");
        assert!(error.contains("stat-file"), "{error}");
        assert!(error.contains("Update or rebuild"), "{error}");
        assert!(!error.contains("unreachable"), "{error}");
    }

    #[test]
    fn signature_mismatch_is_caught_before_guest_start() {
        let engine = Engine::default();
        let component = component(&engine);
        let mut linker = Linker::<()>::new(&engine);
        linker
            .instance("flow-like:node/storage-ops@0.1.0")
            .unwrap()
            .func_wrap("stat-file", |_, (value,): (u32,)| Ok((value,)))
            .unwrap();
        let error = match prepare(&linker, &component) {
            Ok(_) => panic!("wrong signature accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("stat-file"), "{error}");
        assert!(!error.contains("unreachable"), "{error}");
    }

    #[test]
    fn successful_preflight_does_not_execute_guest_start() {
        let engine = Engine::default();
        let component = component(&engine);
        let mut linker = Linker::<()>::new(&engine);
        linker
            .instance("flow-like:node/storage-ops@0.1.0")
            .unwrap()
            .func_wrap("stat-file", |_, (value,): (String,)| Ok((value,)))
            .unwrap();
        let prepared = prepare(&linker, &component).unwrap();
        let mut store = Store::new(&engine, ());
        let error = prepared.instantiate(&mut store).unwrap_err();
        assert!(format!("{error:#}").contains("unreachable"));
    }

    #[test]
    fn legacy_component_does_not_require_storage_ops() {
        let engine = Engine::default();
        let bytes = wat::parse_str(
            r#"(component
            (import "flow-like:node/storage@0.1.0" (instance
                (export "write-file-finish" (func (param "id" string) (result bool))))))"#,
        )
        .unwrap();
        let component = Component::new(&engine, bytes).unwrap();
        let mut linker = Linker::<()>::new(&engine);
        linker
            .instance("flow-like:node/storage@0.1.0")
            .unwrap()
            .func_wrap("write-file-finish", |_, _: (String,)| Ok((true,)))
            .unwrap();
        prepare(&linker, &component).unwrap();
    }

    #[test]
    fn core_import_preflight_checks_signature_without_starting_guest() {
        let engine = Engine::default();
        let bytes = wat::parse_str(
            r#"(module
            (import "flowlike_storage_ops" "request" (func (param i32 i32) (result i64)))
            (func $start unreachable) (start $start))"#,
        )
        .unwrap();
        let module = Module::new(&engine, bytes).unwrap();
        let mut linker = wasmtime::Linker::<()>::new(&engine);
        let missing = linker.instantiate_pre(&module).err().unwrap();
        assert!(host_compatibility_error(missing)
            .to_string()
            .contains("request"));
        linker
            .func_wrap("flowlike_storage_ops", "request", |_: i32| 0i64)
            .unwrap();
        assert!(linker.instantiate_pre(&module).is_err());
        let mut linker = wasmtime::Linker::<()>::new(&engine);
        linker
            .func_wrap("flowlike_storage_ops", "request", |_: i32, _: i32| 0i64)
            .unwrap();
        let prepared = linker.instantiate_pre(&module).unwrap();
        let mut store = Store::new(&engine, ());
        assert!(prepared.instantiate(&mut store).is_err());
    }
}

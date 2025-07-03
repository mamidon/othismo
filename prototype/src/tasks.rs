use async_executor::StaticLocalExecutor;

#[allow(static_mut_refs)] // wasm is single threaded
pub fn executor() -> &'static mut StaticLocalExecutor {
    static mut EXECUTOR: Option<Box<StaticLocalExecutor>> = None;

    unsafe { EXECUTOR.get_or_insert(Box::new(StaticLocalExecutor::new())) }
}

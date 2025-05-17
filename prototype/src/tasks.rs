use core::future::Future;
use core::pin::Pin;

use std::{
    cell::RefCell, collections::{HashMap, VecDeque}, rc::Rc, sync::LazyLock, task::{Context, Poll, RawWaker, RawWakerVTable, Waker}
};

use async_executor::StaticLocalExecutor;


#[allow(static_mut_refs)] // wasm is single threaded
pub fn executor() -> &'static mut StaticLocalExecutor {
    static mut EXECUTOR: Option<Box<StaticLocalExecutor>> = None;

    unsafe { EXECUTOR.get_or_insert(Box::new(StaticLocalExecutor::new())) }
}

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



#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, future::Future, rc::Rc, task::{Poll, Waker}};

    use crate::tasks::executor;

    struct TestTask {
        id: u32,
        wakers: Rc<RefCell<HashMap<u32, Waker>>>
    }

    impl Future for TestTask {
        type Output=();

        fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {

            let waker_exists = self.wakers.borrow().get(&self.id).is_some();

            if (waker_exists) {
                return Poll::Ready(());
            } else {
                self.wakers.borrow_mut().insert(self.id, cx.waker().clone());
                return Poll::Pending;
            }
        }
    }

    #[test]
    fn sleeping_tasks_wait_for_wakup() {
        let executor = executor();
        let wakers = wakers();
        let task_a = TestTask::new(42, wakers.clone());

        let x = executor.spawn(task_a);

        assert!(executor.try_tick());
        assert!(!executor.try_tick());
        
        assert_eq!(wakers.borrow().len(), 1);

        wakers.borrow().get(&42).unwrap().wake_by_ref();

        assert!(executor.try_tick());
        assert!(!executor.try_tick());
    }
/* 
    #[test]
    fn multiple_sleeping_tasks_can_sleep() {
        let wakers = wakers();
        let task_a = TestTask::new(42, wakers.clone());
        let task_b = TestTask::new(52, wakers.clone());

        EXECUTOR.spawn(task_a);
        EXECUTOR.spawn(task_b);

        assert!(EXECUTOR.try_tick());
        assert!(EXECUTOR.try_tick());
        assert!(!EXECUTOR.try_tick());

        assert_eq!(wakers.borrow().len(), 2);

        wakers.borrow().get(&42).unwrap().wake_by_ref();
        
        assert!(EXECUTOR.try_tick());
        assert!(!EXECUTOR.try_tick());


        wakers.borrow().get(&52).unwrap().wake_by_ref();
        
        assert!(EXECUTOR.try_tick());
        assert!(!EXECUTOR.try_tick());
    }

    #[test]
    fn foo() {
        crate::abi::send_message(b"foo");
    }*/

    fn wakers() -> Rc<RefCell<HashMap<u32, Waker>>> {
        Rc::new(RefCell::new(HashMap::new()))
    }

    impl TestTask {
        fn new(id: u32, wakers: Rc<RefCell<HashMap<u32, Waker>>>) -> TestTask {
            TestTask { id, wakers }
        }
    }
}

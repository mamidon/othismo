use core::future::Future;
use core::pin::Pin;

use std::{
    cell::RefCell, collections::{HashMap, VecDeque}, rc::Rc, sync::Arc, task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker}
};

pub struct Task<T> {
    future: Pin<Box<dyn Future<Output = T>>>,
}

impl<T> Task<T> {
    pub fn new(future: impl Future<Output = T> + 'static) -> Task<T> {
        Task {
            future: Box::pin(future),
        }
    }

    pub fn poll(&mut self, ctx: &mut Context) -> Poll<T> {
        self.future.as_mut().poll(ctx)
    }
}

#[derive(Clone)]
pub struct TaskExecutor {
    inner: Rc<RefCell<Executor>>,
}

struct Executor {
    ready: VecDeque<Task<()>>,
    sleeping: HashMap<usize, Task<()>>,
    tasks_started: usize,
    tasks_polled: usize,
    tasks_completed: usize,
}

struct TaskWaker {
    executor: TaskExecutor,
    task_id: usize,
}

impl TaskWaker {
    pub fn new(executor: TaskExecutor, task_id: usize) -> Waker {
        let task_waker = Box::new(TaskWaker { executor, task_id });
        let raw = Box::into_raw(task_waker);

        unsafe {
            Waker::from_raw(RawWaker::new(raw as *const (), &TASK_WAKER_VTABLE))
        }
    }
}

const TASK_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |waker_ptr| {
        let waker = unsafe { &*(waker_ptr as *const TaskWaker) };
        let cloned = Box::new(TaskWaker {
            executor: waker.executor.clone(),
            task_id: waker.task_id
        });
        
        RawWaker::new(Box::into_raw(cloned) as *const (), &TASK_WAKER_VTABLE)
    }
    , |waker_ptr| {
        let waker = unsafe { Box::from_raw(waker_ptr as *mut TaskWaker) };
        waker.executor.wake_task(waker.task_id);

        drop(waker)
    }, |waker_ptr| {
        let waker = unsafe { &*(waker_ptr as *const TaskWaker) };
        waker.executor.wake_task(waker.task_id);
    }, |waker_ptr| {
        unsafe {
            drop(Box::from_raw(waker_ptr as *mut TaskWaker))
        }
    }
);

impl TaskExecutor {
    pub fn new() -> TaskExecutor {
        TaskExecutor {
            inner: Rc::new(RefCell::new(Executor {
                ready: VecDeque::new(),
                sleeping: HashMap::new(),
                tasks_started: 0,
                tasks_polled: 0,
                tasks_completed: 0,
            })),
        }
    }

    pub fn spawn(&mut self, future: impl Future<Output = ()> + 'static) {
        let mut inner = self.inner.borrow_mut();
        inner.ready.push_back(Task::new(future));
    }

    pub fn poll(&mut self) {
        let mut inner = self.inner.borrow_mut();

        if let Some(mut task) = inner.ready.pop_front() {
            inner.tasks_polled += 1;
            let task_id = inner.tasks_polled;
            let waker = TaskWaker::new(self.clone(), task_id);

            let mut context = Context::from_waker(&waker);

            match task.poll(&mut context) {
                Poll::Ready(_) => inner.tasks_completed += 1,
                Poll::Pending => { inner.sleeping.insert(task_id, task); },
            }
        }
    }

    pub(self) fn wake_task(&self, task_id: usize) {
        let mut inner = self.inner.borrow_mut();

        if let Some(task) = inner.sleeping.remove(&task_id) {
            inner.ready.push_back(task);
        }
    }

    pub(self) fn tasks_completed(&self) -> usize {
        self.inner.borrow().tasks_completed
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, future::Future, rc::Rc, task::{Poll, Waker}};

    use super::TaskExecutor;
    
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
        let mut executor = TaskExecutor::new();
        let wakers = wakers();
        let task_a = TestTask::new(42, wakers.clone());

        executor.spawn(task_a);

        executor.poll();

        assert_eq!(executor.tasks_completed(), 0);
        assert_eq!(wakers.borrow().len(), 1);

        wakers.borrow().get(&42).unwrap().wake_by_ref();

        executor.poll();

        assert_eq!(executor.tasks_completed(), 1);
    }

    #[test]
    fn multiple_sleeping_tasks_can_sleep() {
        let mut executor = TaskExecutor::new();
        let wakers = wakers();
        let task_a = TestTask::new(42, wakers.clone());
        let task_b = TestTask::new(52, wakers.clone());

        executor.spawn(task_a);
        executor.spawn(task_b);

        executor.poll();
        executor.poll();
        executor.poll();

        assert_eq!(executor.tasks_completed(), 0);
        assert_eq!(wakers.borrow().len(), 2);

        wakers.borrow().get(&42).unwrap().wake_by_ref();
        executor.poll();

        assert_eq!(executor.tasks_completed(), 1);

        wakers.borrow().get(&52).unwrap().wake_by_ref();
        executor.poll();
    }

    fn wakers() -> Rc<RefCell<HashMap<u32, Waker>>> {
        Rc::new(RefCell::new(HashMap::new()))
    }

    impl TestTask {
        fn new(id: u32, wakers: Rc<RefCell<HashMap<u32, Waker>>>) -> TestTask {
            TestTask { id, wakers }
        }
    }
}

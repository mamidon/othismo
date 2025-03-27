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
            let waker = TaskWaker::new(self.clone(), inner.tasks_polled);

            let mut context = Context::from_waker(&waker);

            match task.poll(&mut context) {
                Poll::Ready(_) => inner.tasks_completed += 1,
                Poll::Pending => inner.ready.push_back(task),
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
    use super::TaskExecutor;

    #[test]
    fn can_test_async() {
        let mut executor = TaskExecutor::new();

        executor.spawn(async {});

        // TODO 
        // Implement a type to hold the state of sending a message to WASM host and awaiting the response
        // That type needs to be tied into the send & receive message sys calls
        // 
        // I think Mailboxes need to store this intermediate state of which messages are incoming & outgoing
        // Such that when the host calls back into us, we do a little bit of book keeping & then defer to the executor
        // Until the user's code terminates (returns a response) or yields (sends a message).
        // 
        // At which point we do some more book keeping and call back into the host.
        // I think this means we need to include an int ID with the message buffer... unless the message buffer ptr
        // can serve as that ID? hmm...
        assert_eq!(executor.tasks_completed(), 1);
    }
}

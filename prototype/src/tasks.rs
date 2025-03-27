use core::future::Future;
use core::pin::Pin;
use std::collections::VecDeque;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

pub struct Task<T> {
    future: Pin<Box<dyn Future<Output = T>>>
}

impl<T> Task<T> {
    pub fn new(future: impl Future<Output = T> + 'static) -> Task<T> {
        Task {
            future: Box::pin(future)
        }
    }

    pub fn poll(&mut self, ctx: &mut Context) -> Poll<T> {
        self.future.as_mut().poll( ctx)
    }
}


pub struct Executor {
    queue: VecDeque<Task<()>>
}

impl Executor {
    pub fn new() -> Executor {
        Executor {
            queue: VecDeque::new()
        }
    }

    pub fn spawn(&mut self, future: impl Future<Output = ()> + 'static) {
        self.queue.push_back(Task::new(future));
    }


    pub fn run(&mut self) {
        while let Some(mut task) = self.queue.pop_front() {
            let waker = dummy_waker();
            let mut context = Context::from_waker(&waker);

            match task.poll(&mut context) {
                Poll::Ready(_) => {},
                Poll::Pending => self.queue.push_back(task),
            }
        }
    }
}


fn raw_waker() -> RawWaker {
    fn noop(_: *const ()) -> () {}
    fn clone(_: *const()) -> RawWaker {
        raw_waker()
    }

    let vtable = &RawWakerVTable::new(clone,noop, noop, noop);
    RawWaker::new(0 as *const (), vtable)
}

fn dummy_waker() -> Waker {
    
    unsafe {
        Waker::from_raw(raw_waker())
    }
}


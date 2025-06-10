use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex, Weak,
    },
};

use tokio::task::JoinHandle;

use super::{Channel, Message, Process, ProcessCtx, ProcessExecutor};


impl Process {
    pub fn start<E: ProcessExecutor>(ctx: ProcessCtx) -> Self {
        let waker_slot = ctx.get_waker_slot();
        let handle = tokio::spawn(E::start(ctx));

        Process { handle, waker: None, waker_slot }
    }
}

/*
The Namespace has a set of processes which can receive messages.
Each process gets a handle to the Namespace's mail box (mpsc ?).
The Namespace takes care of dispatching messages to the correct recipient.
 */

#[derive(Clone)]
pub struct Namespace(Arc<RefCell<InnerNamespace>>);
pub struct InnerNamespace {
    name_space: HashMap<String, Box<Process>>,
    dispatch: Channel<Message>,
    next_handle: AtomicU64,
}

impl Namespace {
    pub fn new() -> Namespace {
        let inner = InnerNamespace {
            name_space: HashMap::new(),
            dispatch: Channel::new(),
            next_handle: AtomicU64::new(1),
        };

        Namespace(Arc::new(RefCell::new(inner)))
    }

    pub fn create_process<E: ProcessExecutor>(&mut self, name: &str) -> () {
        self.0.borrow_mut().create_process::<E>(name)
    }

    pub async fn message_loop(&mut self) -> () {
        loop {
            match self.0.borrow_mut().dispatch.rx.recv().await {
                Some(message) => {
                    // 1. find the target destination
                    // 2. find the target Process, if any
                    // 3. acquire the tx for said process
                    // 4. tx the message
                    // 5. wake the Process
                },
                None => {}
            }
        }
    }
}

impl InnerNamespace {
    pub fn create_process<E: ProcessExecutor>(&mut self, name: &str) -> () {
        let (inbox_tx, inbox_rx) = Channel::new().split();
        let outbox_tx = self.dispatch.tx.clone();

        let ctx = ProcessCtx {
            inbox: inbox_rx,
            outbox: outbox_tx,
            waker_slot: Arc::new(Mutex::new(Option::None))
        };

        assert!(!self.name_space.contains_key(name));

        self.name_space.insert(name.to_string(), Box::new(Process::start::<E>(ctx)));
    }
    
}

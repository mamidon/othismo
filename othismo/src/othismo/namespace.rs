use std::sync::Mutex;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Weak,
    },
};

use bson::Document;
use dashmap::DashMap;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use super::{Channel, Message, Process, ProcessCtx, ProcessExecutor};


impl Process {
    pub fn start<E: ProcessExecutor>(ctx: ProcessCtx, inbox_tx: UnboundedSender<Message>) -> Self {
        let waker_slot = ctx.get_waker_slot();
        let handle = tokio::spawn(async move {
            println!("starting process...");
            E::start(ctx).await;
            println!("endinmg process...");
        });
        
        Process { inbox_tx, handle, waker: None, waker_slot }
    }
}

/*
The Namespace has a set of processes which can receive messages.
Each process gets a handle to the Namespace's mail box (mpsc ?).
The Namespace takes care of dispatching messages to the correct recipient.
 */

pub struct Namespace {
    processes: Arc<DashMap<String, Box<Process>>>,
    dispatch_tx: UnboundedSender<Message>
}

struct NamespaceRouter {
    processes: Arc<DashMap<String, Box<Process>>>,
    dispatch_rx: UnboundedReceiver<Message>
}

impl Namespace {
    pub fn new() -> Namespace {
        let (tx, rx) = Channel::new().split();
        let processes = Arc::new(DashMap::new());

        let namespace = Namespace {
            processes: processes.clone(),
            dispatch_tx: tx
        };

        let mut router = NamespaceRouter {
            processes,
            dispatch_rx: rx
        };

        tokio::spawn(router.message_loop());

        namespace
    }

    pub fn create_process<E: ProcessExecutor>(&mut self, name: &str) -> () {
        let (inbox_tx, mut inbox_rx) = Channel::new().split();
        let outbox_tx = self.dispatch_tx.clone();

        let ctx = ProcessCtx {
            inbox: inbox_rx,
            outbox: outbox_tx,
            waker_slot: Arc::new(Mutex::new(Option::None))
        };

        assert!(!self.processes.contains_key(name));

        self.processes.insert(name.to_string(), Box::new(Process::start::<E>(ctx, inbox_tx)));
    }

    pub fn send_document(&self, destination: &str, document: Document) {
        let mut buffer = Vec::new();
        document.to_writer(&mut buffer);
        self.send_message(destination, Message::new(buffer));
    }

    pub fn send_message(&self, destination: &str, message: Message) {
        self.dispatch_tx.send(message).unwrap()
    }
}

impl NamespaceRouter {
    async fn message_loop(mut self) -> () {
        loop {
            println!("Foo");
            match self.dispatch_rx.recv().await {
                Some(message) => {
                    let document = message.to_bson();
                    let destination = document.get_document("othismo")
                        .and_then(|document| document.get_str("send_to"))
                        .unwrap_or("unknown");

                    let process = self.processes.get(destination)
                        .or_else(|| self.processes.get("/"))
                        .expect("No top level process exists");
                    
                    if process.handle.is_finished() {
                        unsafe {
                            println!("process finished {:?}", process.handle)
                        }
                    }

                    process.inbox_tx.send(message).expect("Failed to send message to process");

                    if let Some(waker) = &process.waker {
                        waker.wake_by_ref();
                    }
                },
                None => {}
            }
        }
    }
}
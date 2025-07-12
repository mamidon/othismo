use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

use crate::othismo::executors::{ConsoleExecutor, InstanceExecutor};
use crate::othismo::image::Image;

use super::{Channel, Message, Process, ProcessCtx, ProcessExecutor};

impl Process {
    pub fn start<E: ProcessExecutor>(
        executor: E,
        ctx: ProcessCtx,
        inbox_tx: UnboundedSender<Message>,
    ) -> Self {
        let waker_slot = ctx.get_waker_slot();
        let handle = tokio::spawn(executor.start(ctx));

        Process {
            inbox_tx,
            handle,
            waker: None,
            waker_slot,
        }
    }
}

/*
Each process gets a handle to the Namespace's mail box (mpsc ?).
The Namespace takes care of dispatching messages to the correct recipient.
 */

pub struct Namespace {
    processes: Arc<DashMap<String, Box<Process>>>,
    dispatch_tx: UnboundedSender<Message>,
    messages_sent: Arc<AtomicU64>,
    last_message_sent_at: Arc<AtomicU64>,
}

struct NamespaceRouter {
    processes: Arc<DashMap<String, Box<Process>>>,
    dispatch_rx: UnboundedReceiver<Message>,
}

impl Namespace {
    pub fn new() -> Namespace {
        let (tx, rx) = Channel::new().split();
        let processes = Arc::new(DashMap::new());

        let mut router = NamespaceRouter {
            processes: processes.clone(),
            dispatch_rx: rx,
        };

        let mut namespace = Namespace {
            processes: processes,
            dispatch_tx: tx,
            messages_sent: Arc::new(AtomicU64::new(0)),
            last_message_sent_at: Arc::new(AtomicU64::new(0)),
        };

        tokio::spawn(router.message_loop());

        namespace
    }

    pub fn create_process<E: ProcessExecutor>(&mut self, executor: E, name: &str) -> () {
        let (inbox_tx, mut inbox_rx) = Channel::new().split();
        let outbox_tx = self.dispatch_tx.clone();

        let ctx = ProcessCtx {
            inbox: inbox_rx,
            outbox: outbox_tx,
            waker_slot: Arc::new(Mutex::new(Option::None)),
        };

        assert!(!self.processes.contains_key(name));
        let process = Box::new(Process::start(executor, ctx, inbox_tx));
        self.processes.insert(name.to_string(), process);
    }

    pub fn send_document(&self, destination: &str, document: Document) {
        let mut buffer = Vec::new();
        document.to_writer(&mut buffer);
        self.send_message(destination, Message::new(buffer));
    }

    pub fn send_message(&self, destination: &str, message: Message) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.messages_sent.fetch_add(1, Ordering::SeqCst);
        self.last_message_sent_at.store(now, Ordering::SeqCst);

        self.dispatch_tx.send(message).unwrap()
    }

    pub async fn wait_for_idleness(&self, duration: Duration) -> () {
        let idle_timeout = Duration::from_secs(10);
        let started = SystemTime::now();

        loop {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let idleness =
                Duration::from_secs(now - self.last_message_sent_at.load(Ordering::SeqCst));

            if idleness > idle_timeout {
                return;
            }

            if started.elapsed().unwrap() > duration {
                return;
            }

            tokio::time::sleep(Duration::from_secs(2)).await
        }
    }
}

impl NamespaceRouter {
    async fn message_loop(mut self) -> () {
        loop {
            println!("namespace_router ... loop");
            match self.dispatch_rx.recv().await {
                Some(message) => {
                    println!("namespace_router ... message received");
                    let document = message.to_bson();
                    let destination = document
                        .get_document("othismo")
                        .and_then(|document| document.get_str("send_to"))
                        .unwrap_or("unknown");

                    let process = self
                        .processes
                        .get(destination)
                        .or_else(|| self.processes.get("/"))
                        .expect("No top level process exists");

                    if process.handle.is_finished() {
                        let (k, v) = self.processes.remove("/").unwrap();
                        println!("waiting for error...");
                        v.handle
                            .await
                            .inspect_err(|e| println!("This killed the process {}", e));
                    }

                    process
                        .inbox_tx
                        .send(message)
                        .expect("Failed to send message to process");

                    if let Some(waker) = &process.waker {
                        waker.wake_by_ref();
                    }
                }
                None => {
                    println!("namespace_router ... no message received");
                }
            }
        }
    }
}

impl From<Image> for Namespace {
    fn from(image: Image) -> Self {
        let mut namespace = Namespace::new();
        let names = image.list_objects("").unwrap();

        namespace.create_process(ConsoleExecutor, "/");

        for name in image.list_objects("").unwrap() {
            let object = image.get_object(&name).unwrap();

            match object {
                super::image::Object::Instance(instance) => {
                    println!("starting executor for ... {}", &name);

                    let executor: InstanceExecutor = instance.into();
                    namespace.create_process(executor, &name);
                }
                _ => {}
            }
        }
        namespace
    }
}

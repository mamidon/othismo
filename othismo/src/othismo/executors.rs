use crate::othismo;
use crate::othismo::image::InstanceAtRest;
use bson::{doc, to_bson, Document};
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use tokio::sync::mpsc::error::TryRecvError;
use wasmer::{
    imports, Function, FunctionEnv, FunctionEnvMut, Instance, Memory, Store, TypedFunction,
};

use super::{Message, ProcessCtx, ProcessExecutor};

pub struct ConsoleExecutor;
pub struct ConsoleTask {
    ctx: ProcessCtx,
}

impl ProcessExecutor for ConsoleExecutor {
    fn start(self, ctx: ProcessCtx) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(ConsoleTask { ctx })
    }
}

impl Future for ConsoleTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.ctx.fill_waker_slot(cx.waker().clone());

        println!("Polling Console");
        return match this.ctx.inbox.poll_recv(cx) {
            Poll::Ready(Some(message)) => {
                let document = Document::from_reader(&mut message.bytes.as_slice()).unwrap();
                println!("{}", document);
                print!("...pending, message");
                Poll::Pending
            }
            Poll::Ready(None) => {
                println!("...ready");
                Poll::Ready(())
            }
            Poll::Pending => {
                println!("...pending, no message");
                Poll::Pending
            }
        };
    }
}

pub struct EchoExecutor;
pub struct EchoTask {
    ctx: ProcessCtx,
}

impl ProcessExecutor for EchoExecutor {
    fn start(
        self,
        ctx: ProcessCtx,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(EchoTask { ctx })
    }
}

impl Future for EchoTask {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.ctx.fill_waker_slot(cx.waker().clone());
        loop {
            println!("EchoExecutor polled");
            match this.ctx.inbox.try_recv() {
                Ok(message) => {
                    let document = Document::from_reader(&mut message.bytes.as_slice()).unwrap();
                    let othismo = document.get_document("othismo").unwrap();
                    let reply_to = othismo.get_str("reply_to").unwrap();
                    let response_id = othismo.get_i64("request_id").unwrap();

                    let mut response = doc! {
                        "othismo": doc! {
                            "send_to": reply_to,
                            "response_id": response_id
                        }
                    };

                    for (k, v) in document.iter().filter(|(k, v)| *k != "othismo") {
                        response.insert(k, v);
                    }

                    let mut buffer = Vec::new();

                    response.to_writer(&mut buffer).unwrap();

                    this.ctx.outbox.send(Message { bytes: buffer });
                }
                Err(reason) => match reason {
                    TryRecvError::Empty => return Poll::Pending,
                    TryRecvError::Disconnected => return Poll::Ready(()),
                },
            }
            println!("EchoExecutor exited");
        }
    }
}

pub struct InstanceExecutor {
    instance_at_rest: InstanceAtRest,
}
pub struct InstanceTask {
    ctx: ProcessCtx,
    instance: Instance,
    store: Store,
}
pub struct InstanceEnv {
    memory: Option<Memory>,
    task: Option<InstanceTask>,
}

impl ProcessExecutor for InstanceExecutor {
    fn start(self, context: ProcessCtx) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut store = Store::default();
        let buffer = self.instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(&mut store, &buffer).unwrap();
        let env = FunctionEnv::new(
            &mut store,
            InstanceEnv {
                memory: None,
                task: None,
            },
        );

        let send_message_trampoline =
            Function::new_typed_with_env(&mut store, &env, native_trampolines::send_message);

        let wasmer_instance = wasmer::Instance::new(
            &mut store,
            &wasmer_instance_module,
            &imports! {
                "othismo" => {
                    "_send_message" => send_message_trampoline,
                }
            },
        )
        .unwrap();

        env.as_mut(&mut store).memory = Some(
            wasmer_instance
                .exports
                .get_memory("memory")
                .unwrap()
                .clone(),
        );

        let task = Box::pin(InstanceTask {
            ctx: context,
            instance: wasmer_instance,
            store,
        });

        task
    }
}

impl InstanceTask {
    pub fn send_message(mut env: FunctionEnvMut<InstanceEnv>, head: u32, length: u32) -> u32 {
        println!("native::send_message({}, {})", head, length);

        let (environment, mut store) = env.data_and_store_mut();
        let view = environment.memory.as_mut().unwrap().view(&store);
        let mut buffer: Vec<u8> = vec![0; length as usize];
        view.read(head as u64, buffer.as_mut_slice());

        println!(
            "\"{}\"",
            String::from_utf8(buffer).unwrap_or("bad_utf8".to_string())
        );

        return 0;
    }

    pub fn receive_message(&mut self, message: &[u8]) -> othismo::Result<()> {
        let allocate_message: TypedFunction<u32, u64> = self
            .instance
            .exports
            .get_function("allocate_message")?
            .typed(&self.store)?;

        let message_received: TypedFunction<(u32, u32), ()> = self
            .instance
            .exports
            .get_function("message_received")?
            .typed(&self.store)?;

        let packed_tuple = allocate_message.call(&mut self.store, message.len() as u32)?;
        let message_handle = (packed_tuple >> 32) as u32;
        let message_buffer_ptr = (packed_tuple << 32) >> 32;

        println!("handle: {}, ptr: {}", message_handle, message_buffer_ptr);

        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(&self.store);

        view.write(message_buffer_ptr, message);

        message_received.call(
            &mut self.store,
            message_buffer_ptr as u32,
            message.len() as u32,
        )?;

        Ok(())
    }
}

impl Future for InstanceTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        println!("Polling instance");
        match this.ctx.inbox.poll_recv(cx) {
            Poll::Ready(Some(message)) => {
                this.receive_message(&message.bytes).unwrap();
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(()),
            Poll::Pending => {
                println!("... pending, no message");
                Poll::Pending
            }
        }
    }
}

mod native_trampolines {
    use wasmer::{AsStoreMut, FunctionEnvMut};

    use crate::othismo::Message;

    use super::InstanceEnv;

    pub fn send_message(mut env: FunctionEnvMut<InstanceEnv>, head: u32, length: u32) -> u32 {
        println!("native::send_message({}, {})", head, length);

        let (environment, mut store) = env.data_and_store_mut();
        let view = environment.memory.as_mut().unwrap().view(&store);
        let mut buffer: Vec<u8> = vec![0; length as usize];
        view.read(head as u64, buffer.as_mut_slice());

        environment
            .task
            .as_ref()
            .unwrap()
            .ctx
            .outbox
            .send(Message::new(buffer));

        return 0;
    }
}

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use tokio::sync::mpsc::error::TryRecvError;
use bson::{doc, to_bson, Document};

use super::{Message, ProcessCtx, ProcessExecutor};

pub struct ConsoleExecutor {
    ctx: ProcessCtx
}

impl ProcessExecutor for ConsoleExecutor {
    fn start(ctx: ProcessCtx) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(ConsoleExecutor {
            ctx
        })
    }
}

impl Future for ConsoleExecutor {
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
            },
            Poll::Ready(None) => {
                println!("...ready");
                Poll::Ready(())
            },
            Poll::Pending => {
                println!("...pending, no message");
                Poll::Pending
            }
        }
    }
}

impl Drop for ConsoleExecutor {
    fn drop(&mut self) {
        println!("ConsoleExecutor DROPPED")
    }
}

pub struct EchoExecutor {
    ctx: ProcessCtx
}

impl ProcessExecutor for EchoExecutor {
    fn start(ctx: ProcessCtx) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(EchoExecutor {
            ctx
        })
    }
}

impl Future for EchoExecutor {
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
                },
                Err(reason) => {
                    match reason {
                        TryRecvError::Empty => { return Poll::Pending },
                        TryRecvError::Disconnected => { return Poll::Ready(()) }
                    }
                }
            }
            println!("EchoExecutor exited");
        }
    }
}
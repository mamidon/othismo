use std::io::BufWriter;

use crate::othismo::image::{Image, InstanceAtRest, Object};
use crate::othismo::{Errors, OthismoError, Result};
use wasmer::{
    imports, AsStoreMut, Function, FunctionEnv, Global, Imports, Instance, Memory, MemoryView,
    Store, TypedFunction, Value,
};

struct Environment {
    instance: Option<ExecutionSession>,
    memory: Option<Memory>,
}

#[derive(Clone)]
struct ExecutionSession {
    instance_at_rest: InstanceAtRest,
    instance: Instance,
}

impl ExecutionSession {
    pub fn from_instance_at_rest(
        store: &mut Store,
        instance_at_rest: InstanceAtRest,
    ) -> Result<ExecutionSession> {
        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        // todo create a context which stores a reference to the memory
        let env = FunctionEnv::new(
            store,
            Environment {
                memory: None,
                instance: None,
            },
        );
        let send_message_trampoline =
            Function::new_typed_with_env(store, &env, native::send_message);
        let cast_message_trampoline =
            Function::new_typed_with_env(store, &env, native::cast_message);

        let wasmer_instance = wasmer::Instance::new(
            store,
            &wasmer_instance_module,
            &imports! {
                "othismo" => {
                    "_send_message" => send_message_trampoline,
                    "_cast_message" => cast_message_trampoline
                }
            },
        )?;

        let instance_session = ExecutionSession {
            instance_at_rest,
            instance: wasmer_instance.clone(),
        };

        env.as_mut(store).memory = Some(
            wasmer_instance
                .exports
                .get_memory("memory")
                .unwrap()
                .clone(),
        );
        env.as_mut(store).instance = Some(instance_session.clone());

        if let Some((name, desc)) = instance_session
            .instance
            .exports
            .iter()
            .find(|e| e.0.eq("_othismo_start"))
        {
            println!("invoking _othismo_start");
            instance_session.call_othismo_start(store)?;
        }

        Ok(instance_session)
    }

    pub fn into_instance_at_rest(mut self, store: &mut Store) -> Result<InstanceAtRest> {
        for (name, value) in &self.instance.exports {
            if let wasmer::Extern::Global(global) = &value {
                self.instance_at_rest
                    .set_exported_global(&name, global.get(store))?;
            }

            if let wasmer::Extern::Memory(memory) = &value {
                self.instance_at_rest.clear_data_segments();

                let page_size_in_bytes = 64;
                let view = memory.view(store);
                let mut buffer: Vec<u8> = std::iter::repeat(0).take(page_size_in_bytes).collect();

                let mut skipped = 0;
                let mut persisted = 0;

                for index in 0..(view.data_size() / page_size_in_bytes as u64) {
                    let offset = index * page_size_in_bytes as u64;
                    view.read(offset, &mut buffer)?;

                    if (buffer.iter().all(|&byte| byte == 0)) {
                        skipped += 1;
                        continue;
                    }

                    persisted += 1;
                    self.instance_at_rest
                        .add_data_segment(offset as i32, &buffer);
                }

                // Possibly resize memory section to ensure we have enough memory
                self.instance_at_rest.resize_memory(view.data_size())?;
            }
        }

        Ok(self.instance_at_rest)
    }

    pub fn receive_message(
        &self,
        store: &mut impl AsStoreMut,
        message: (u32, &[u8]),
    ) -> Result<()> {
        let allocate_message: TypedFunction<u32, u64> = self
            .instance
            .exports
            .get_function("allocate_message")?
            .typed(store)?;

        let message_received: TypedFunction<(u32, u32), ()> = self
            .instance
            .exports
            .get_function("message_received")?
            .typed(store)?;

        let packed_tuple = allocate_message.call(store, message.1.len() as u32)?;
        let message_handle = (packed_tuple >> 32) as u32;
        let message_buffer_ptr = (packed_tuple << 32) >> 32;

        println!("handle: {}, ptr: {}", message_handle, message_buffer_ptr);

        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(store);

        view.write(message_buffer_ptr, message.1);

        message_received.call(store, message_handle, message.0)?;

        Ok(())
    }

    pub fn call_othismo_start(&self, store: &mut Store) -> Result<()> {
        let othismo_start: TypedFunction<(), ()> = self
            .instance
            .exports
            .get_function("_othismo_start")?
            .typed(store)?;

        let result = othismo_start.call(store)?;

        let memory = self.instance.exports.get_memory("memory")?;

        Ok(())
    }
}

pub fn send_message(image: &mut Image, instance_name: &str) -> Result<()> {
    let object = image.get_object(instance_name)?;
    let instance_at_rest = match object {
        Object::Instance(instance_at_rest) => instance_at_rest,
        Object::Module(_) => Err(OthismoError::ObjectDoesNotExist)?,
    };

    let mut store = Store::default();
    let instance_session = ExecutionSession::from_instance_at_rest(&mut store, instance_at_rest)?;

    instance_session.receive_message(&mut store, (0, b"Hello, world"))?;

    let mut dehydrated_instance = instance_session.into_instance_at_rest(&mut store)?;
    image.remove_object(instance_name)?;
    image.import_object(instance_name, Object::Instance(dehydrated_instance))?;
    Ok(())
}

mod native {
    use wasmer::{AsStoreMut, FunctionEnvMut};

    use super::Environment;

    pub fn send_message(
        mut env: FunctionEnvMut<Environment>,
        handle: u32,
        head: u32,
        length: u32,
    ) -> u32 {
        println!("native::send_message({}, {}, {})", handle, head, length);

        let (environment, mut store) = env.data_and_store_mut();
        let view = environment.memory.as_mut().unwrap().view(&store);
        let mut buffer: Vec<u8> = vec![0; length as usize];
        view.read(head as u64, buffer.as_mut_slice());

        println!(
            "\"{}\"",
            String::from_utf8(buffer).unwrap_or("bad_utf8".to_string())
        );

        environment
            .instance
            .as_mut()
            .unwrap()
            .receive_message(&mut store, (1, b"Response"));

        return 0;
    }

    pub fn cast_message(
        mut env: FunctionEnvMut<Environment>,
        handle: u32,
        head: u32,
        length: u32,
    ) -> u32 {
        println!("native::cast_message({}, {}, {})", handle, head, length);

        let (environment, store) = env.data_and_store_mut();
        let view = environment
            .memory
            .as_mut()
            .expect("Native functions need access to linear memory")
            .view(&store);
        let mut buffer: Vec<u8> = vec![0; length as usize];
        view.read(head as u64, buffer.as_mut_slice());

        println!(
            "\"{}\"",
            String::from_utf8(buffer).unwrap_or("bad_utf8".to_string())
        );

        return 0;
    }
}

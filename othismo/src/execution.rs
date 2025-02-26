use std::io::BufWriter;

use wasmer::{imports, Function, FunctionEnv, Global, Imports, Instance, Memory, MemoryView, Store, TypedFunction, Value};

use crate::othismo::image::{Image, InstanceAtRest, Object};
use crate::othismo::{Errors, Result, OthismoError};

struct Session<'s> {
    image: &'s mut Image,
    store: Store,
}

struct Environment {
    memory: Option<Memory>
}

struct InstanceSession {
    instance_at_rest: InstanceAtRest,
    instance: Instance
}

impl InstanceSession {
    pub fn from_instance_at_rest(store: &mut Store, instance_at_rest: InstanceAtRest) -> Result<InstanceSession> {
        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;   
        // todo create a context which stores a reference to the memory 
        let env = FunctionEnv::new(store, Environment { memory: None });
        let trampoline = Function::new_typed_with_env(store, &env, native::send_message);

        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &imports! {
            "othismo" => {
                "send_message" => trampoline
            }
        })?;

        env.as_mut(store).memory = Some(wasmer_instance.exports.get_memory("memory").unwrap().clone());
    
        let instance_session = InstanceSession {
            instance_at_rest,
            instance: wasmer_instance
        };

        if let Some((name, desc)) = instance_session.instance.exports.iter().find(|e| e.0.eq("_othismo_start")) {
            println!("invoking _othismo_start");
            instance_session.call_othismo_start(store)?;
        }

        Ok(instance_session)
    }

    pub fn into_instance_at_rest(mut self, store: &mut Store) -> Result<InstanceAtRest> {
        for (name, value) in &self.instance.exports {
            if let wasmer::Extern::Global(global) = &value {
                self.instance_at_rest.set_exported_global(&name, global.get(store))?;
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
                    self.instance_at_rest.add_data_segment(offset as i32, &buffer);
                }

                // Possibly resize memory section to ensure we have enough memory
                self.instance_at_rest.resize_memory(view.data_size())?;
            }
        }
        
        Ok(self.instance_at_rest)
    }

    pub fn call_function(&self, store: &mut Store) -> Result<()> {
        let prepare_inbox: TypedFunction<(u32), (u32)> = self.instance
            .exports
            .get_function("prepare_inbox")?
            .typed(store)?;

        let message_received: TypedFunction<(), ()> = self.instance
            .exports
            .get_function("message_received")?
            .typed(store)?;

        let message = "Hello, world!".to_string();
        let result = prepare_inbox.call(store, message.len() as u32)?;

        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(store);
        view.write(result as u64, message.as_bytes());

        message_received.call(store)?;
        
        Ok(())
    }

    pub fn call_othismo_start(&self, store: &mut Store) -> Result<()> {
        let othismo_start: TypedFunction<(), ()> = self.instance
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
        Object::Module(_) => Err(OthismoError::ObjectDoesNotExist)?
    };

    let mut store = Store::default();
    let instance_session = InstanceSession::from_instance_at_rest(
        &mut store, 
        instance_at_rest
    )?;

    instance_session.call_function(&mut store)?;
    
    let mut dehydrated_instance = instance_session.into_instance_at_rest(&mut store)?;
    image.remove_object(instance_name)?;
    image.import_object(instance_name, Object::Instance(dehydrated_instance))?;
    Ok(())
}

mod native {
    use wasmer::FunctionEnvMut;

    use super::Environment;

    pub fn send_message(mut env: FunctionEnvMut<Environment>, head: u32, length: u32) -> u32 {
        println!("native::send_message({}, {})", head, length);

        let (environment, store) = env.data_and_store_mut();
        let view = environment.memory.as_mut().expect("Native functions need access to linear memory").view(&store);
        let mut buffer: Vec<u8> = vec![0; length as usize];
        view.read(head as u64, buffer.as_mut_slice());
        
        println!("\"{}\"", String::from_utf8(buffer).unwrap());

        return 0;
    }
}
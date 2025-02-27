use std::collections::HashMap;


pub struct Namespace {
    name_space: HashMap<String, Box<dyn Recipient>>
}

impl Namespace {
    pub fn new() -> Namespace { 
        Namespace {
            name_space: HashMap::new()
        }
    }

    pub fn add_recipient(&mut self, name: &str, recipient: impl Recipient + 'static) {
        self.name_space.insert(name.to_string(), Box::new(recipient));
    }

    pub fn remove_recipient(&mut self, name: &str) -> Option<Box<dyn Recipient>> {
        self.name_space.remove(name)
    }

    pub fn send_message(&self, name: &str, message: &Vec<u8>) -> Option<Vec<u8>> {
        let recipient = self.name_space.get(name)?;

        recipient.receive(self, message)
    }    
}

pub trait Recipient {
    fn receive(&self, namespace: &Namespace, message: &Vec<u8>) -> Option<Vec<u8>>;
}

mod tests {
    use super::{Namespace, Recipient};
    use bson::{ bson, to_vec, Bson };
    use wasmer::wasmparser::names;

    fn make_message(bson: Bson) -> Vec<u8> {
        bson::to_vec(&bson).unwrap()
    }

    #[test]
    fn echos_come_back() {
        let mut namespace = Namespace::new();
        let echo = EchoRecipient{};
        let message = make_message(bson!({ "message": "Hello, world!" }));

        namespace.add_recipient("echo", echo);

        
        let response = namespace.send_message("echo", &message);

        assert_eq!(response, Some(message))
    }

    #[test]
    fn messages_can_go_several_layers() {
        let mut namespace = Namespace::new();

        let original_message = make_message(bson!({ "original": "message" }));
        let replacement_message = make_message(bson!({ "replacement": "message" }));
        let captured_message = replacement_message.clone();

        namespace.add_recipient("a", ForwardRecipient("b".to_string()));
        namespace.add_recipient("b", ForwardRecipient("c".to_string()));
        namespace.add_recipient("c", LambdaRecipient(Some("d".to_string()), Box::new(move |message| { Some(captured_message.clone()) })));
        namespace.add_recipient("d", EchoRecipient{});

        let response = namespace.send_message("a", &original_message);

        assert_eq!(response, Some(replacement_message))
    }
    


    struct LambdaRecipient(Option<String>, Box<dyn Fn(&Vec<u8>) -> Option<Vec<u8>>>);
    struct ForwardRecipient(String);
    struct EchoRecipient;
    
    impl Recipient for LambdaRecipient {
        fn receive(&self, namespace: &Namespace, message: &Vec<u8>) -> Option<Vec<u8>> {
            match &self.0 {
                Some(destination) => namespace.send_message(&destination, &self.1(message)?),
                None => self.1(message)
            }
        }
    }

    impl Recipient for ForwardRecipient {
        fn receive(&self, namespace: &Namespace, message: &Vec<u8>) -> Option<Vec<u8>> {
            namespace.send_message(&self.0, message)
        }
    }

    impl Recipient for EchoRecipient {
        fn receive(&self, namespace: &super::Namespace, message: &Vec<u8>) -> Option<Vec<u8>> {
            Some(message.clone())
        }
    }
}
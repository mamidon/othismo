
pub trait NativeModule {
    fn message_received(message: Vec<u8>);
}
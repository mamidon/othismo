
/*
    On receiving a message.
    1. Message is placed into Inbox
    2. message_receive is invoked
    3. future is constructor to process the message & added to the executor
    4. the executor is invoked with a join
    5. the response, if any, is turned into a send future, placed on the executor, and invoked

    On sending a message
    1. guest invokes guest send_message
    2. a future is constructed
 */

 mod tasks;

 mod abi;


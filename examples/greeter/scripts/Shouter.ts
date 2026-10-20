import { Message, Receiver, Sender, bind, emit, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"

export { rt_inbox, rt_deliver }

class Shouter extends Receiver {
  receive(msg: Message, from: Sender): void {
    if (msg instanceof Line) {
      emit(new Line((msg as Line).text.toUpperCase()))
    }
  }
}

bind(new Shouter())

import { Message, Receiver, Sender, bind, emit, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"
import { Echoed } from "../messages"

// The host's entry points into this module.
export { rt_inbox, rt_deliver }

class Echo extends Receiver {
  receive(msg: Message, from: Sender): void {
    if (msg instanceof Line) {
      emit(new Echoed((msg as Line).text))
    }
  }
}

bind(new Echo())

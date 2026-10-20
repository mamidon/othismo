import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line, Echoed } from "../messages"

// The host's entry points into this module.
export { rt_inbox, rt_deliver }

class Echo extends Receiver {
  constructor(private out: OutPort<Echoed>) {
    super()
  }

  receive(msg: Message): void {
    if (msg instanceof Line) {
      this.out.send(new Echoed((msg as Line).text))
    }
  }
}

// One OutPort per `out` in the protocol, in declaration order.
bind(new Echo(new OutPort<Echoed>(0)))

import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"

export { rt_inbox, rt_deliver }

class Shouter extends Receiver {
  constructor(private out: OutPort<Line>) {
    super()
  }

  receive(msg: Message): void {
    if (msg instanceof Line) {
      this.out.send(new Line((msg as Line).text.toUpperCase()))
    }
  }
}

bind(new Shouter(new OutPort<Line>(0)))

use std::convert::TryInto;

use super::frame_writer::FrameWriter;
use super::packet_writer::PacketWriter;
use super::{Code, Error, ProtocolType};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Verdict<'a> {
    Ack,
    Nack(&'a [u8]),
    Rej,
}

pub trait Protocol {
    fn protocol(&self) -> ProtocolType;

    fn own_options(&mut self, p: &mut PacketWriter) -> Result<(), Error>;
    fn own_option_nacked(&mut self, code: u8, data: &[u8], is_rej: bool);

    fn peer_options_start(&mut self);
    fn peer_option_received(&mut self, code: u8, data: &[u8]) -> Verdict;
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum State {
    Closed,
    ReqSent,
    AckReceived,
    AckSent,
    Opened,
}

pub struct StateMachine<P> {
    id: u8,
    state: State,
    proto: P,
}

impl<P: Protocol> StateMachine<P> {
    pub fn new(proto: P) -> Self {
        Self {
            id: 1,
            state: State::Closed,
            proto,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn proto(&mut self) -> &mut P {
        &mut self.proto
    }

    pub fn open(&mut self, w: &mut FrameWriter<'_>) -> Result<(), Error> {
        match self.state {
            State::Closed => {
                self.send_configure_request(w)?;
                self.state = State::ReqSent;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn close(&mut self, _w: &mut FrameWriter<'_>) -> Result<(), Error> {
        self.state = State::Closed;
        Ok(())
    }

    pub fn handle(&mut self, pkt: &mut [u8], w: &mut FrameWriter<'_>) -> Result<(), Error> {
        if pkt.len() < 6 {
            println!("warn: too short");
            return Err(Error::TooShort);
        }
        let code = Code::from(pkt[2]);
        let id = pkt[3];
        let len = u16::from_be_bytes(pkt[4..6].try_into().unwrap()) as usize;
        if len + 2 > pkt.len() {
            println!("warn: len too short");
            return Err(Error::TooShort);
        }
        let pkt = &mut pkt[..len + 2];

        let old_state = self.state;
        match (code, self.state) {
            // reply EchoReq on state Opened, ignore in all other states (including Closed!)
            (Code::EchoReq, State::Opened) => self.send_echo_response(pkt, w)?,
            (Code::EchoReq, x) => println!("WARNING: unexpected EchoReq in state {:?}", x),

            // in state Closed, reply to any packet with TerminateAck (except to EchoReq!)
            (_, State::Closed) => self.send_terminate_ack(id, w)?,

            (Code::ConfigureReq, _) => {
                let acked = self.received_configure_req(pkt, w)?;
                match (acked, self.state) {
                    (_, State::Closed) => unreachable!(),
                    (true, State::ReqSent) => self.state = State::AckSent,
                    (true, State::AckReceived) => self.state = State::Opened,
                    (true, State::AckSent) => self.state = State::AckSent,
                    (true, State::Opened) => {
                        self.send_configure_request(w)?;
                        self.state = State::AckSent;
                    }
                    (false, State::AckSent) => self.state = State::ReqSent,
                    (false, State::Opened) => {
                        self.send_configure_request(w)?;
                        self.state = State::ReqSent;
                    }
                    (false, _) => {}
                }
            }

            (Code::ConfigureAck, State::ReqSent) => self.state = State::AckReceived,
            (Code::ConfigureAck, State::AckSent) => self.state = State::Opened,
            (Code::ConfigureAck, State::AckReceived) | (Code::ConfigureAck, State::Opened) => {
                self.send_configure_request(w)?;
                self.state = State::ReqSent;
            }

            (Code::ConfigureNack, _) | (Code::ConfigureRej, _) => {
                let is_rej = code == Code::ConfigureRej;

                parse_options(pkt, |code, data| {
                    self.proto.own_option_nacked(code, data, is_rej);
                    Ok(())
                })?;

                self.send_configure_request(w)?;
                match self.state {
                    State::Closed => unreachable!(),
                    State::AckSent => {}
                    _ => self.state = State::ReqSent,
                }
            }

            x => println!("WARNING: unexpected packet {:?} state {:?}", x, self.state),
        }

        if old_state != self.state {
            println!(
                "PPP {:?} state {:?} -> {:?}",
                self.proto.protocol(),
                old_state,
                self.state
            );
        }

        Ok(())
    }

    fn next_id(&mut self) -> u8 {
        self.id = self.id.wrapping_add(1);
        self.id
    }

    fn send_configure_request(&mut self, w: &mut FrameWriter<'_>) -> Result<(), Error> {
        let mut p = PacketWriter::new();
        self.proto.own_options(&mut p)?;
        p.write(w, self.proto.protocol(), Code::ConfigureReq, self.next_id())
    }

    fn send_terminate_request(
        &mut self,
        reason: &[u8],
        w: &mut FrameWriter<'_>,
    ) -> Result<(), Error> {
        let mut p = PacketWriter::new();
        p.append(reason)?;
        p.write(w, self.proto.protocol(), Code::TerminateReq, self.next_id())
    }

    fn send_terminate_ack(&mut self, id: u8, w: &mut FrameWriter<'_>) -> Result<(), Error> {
        let mut p = PacketWriter::new();
        p.write(w, self.proto.protocol(), Code::TerminateAck, id)
    }

    fn send_code_reject(&mut self, pkt: &[u8], w: &mut FrameWriter<'_>) -> Result<(), Error> {
        let mut p = PacketWriter::new();
        p.append(&pkt[2..])?; // don't include proto
        p.write(w, self.proto.protocol(), Code::CodeRej, self.next_id())
    }

    fn send_echo_response(&mut self, pkt: &mut [u8], w: &mut FrameWriter<'_>) -> Result<(), Error> {
        pkt[2] = Code::EchoReply.into();
        w.start()?;
        w.append(pkt)?;
        w.finish()
    }

    // TODO maybe this should be in PPP because it's only for LCP
    pub fn send_protocol_reject(
        &mut self,
        pkt: &[u8],
        w: &mut FrameWriter<'_>,
    ) -> Result<(), Error> {
        let mut p = PacketWriter::new();
        p.append(pkt)?;
        p.write(w, self.proto.protocol(), Code::ProtocolRej, self.next_id())
    }

    fn received_configure_req(
        &mut self,
        pkt: &[u8],
        w: &mut FrameWriter<'_>,
    ) -> Result<bool, Error> {
        let id = pkt[3];

        let mut p = PacketWriter::new();
        let mut code = Code::ConfigureAck;

        self.proto.peer_options_start();
        parse_options(pkt, |ocode, data| {
            let (ret_code, data) = match self.proto.peer_option_received(ocode, data) {
                Verdict::Ack => (Code::ConfigureAck, data),
                Verdict::Nack(data) => (Code::ConfigureNack, data),
                Verdict::Rej => (Code::ConfigureRej, data),
            };

            if code < ret_code {
                code = ret_code;
                p.reset();
            }

            if code == ret_code {
                p.append_option(ocode, data)?;
            }

            Ok(())
        })?;

        p.write(w, self.proto.protocol(), code, id)?;
        Ok(code == Code::ConfigureAck)
    }
}

fn parse_options(
    mut pkt: &[u8],
    mut f: impl FnMut(u8, &[u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    if pkt.len() < 6 {
        return Err(Error::TooShort);
    }
    pkt = &pkt[6..]; // skip header

    while pkt.len() != 0 {
        if pkt.len() < 2 {
            return Err(Error::TooShort);
        }

        let code = pkt[0];
        let len = pkt[1] as usize;

        if pkt.len() < len {
            return Err(Error::TooShort);
        }
        if len < 2 {
            return Err(Error::TooShort);
        }

        let data = &pkt[2..len];
        f(code, data)?;
        pkt = &pkt[len..];
    }

    Ok(())
}
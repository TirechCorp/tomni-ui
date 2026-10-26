//! O `Device` do `smoltcp` sobre filas em memória: os pacotes IP entram e saem por aqui, sem
//! camada de enlace (`Medium::Ip`).

use std::collections::VecDeque;

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

pub(crate) struct QueueDevice {
    mtu: usize,
    /// Os pacotes que chegaram do túnel e a pilha ainda não leu.
    pub(crate) rx: VecDeque<Vec<u8>>,
    /// Os pacotes que a pilha montou e ainda não foram para o túnel.
    pub(crate) tx: Vec<Vec<u8>>,
}

impl QueueDevice {
    pub(crate) fn new(mtu: usize) -> Self {
        Self {
            mtu,
            rx: VecDeque::new(),
            tx: Vec::new(),
        }
    }
}

pub(crate) struct Rx(Vec<u8>);

pub(crate) struct Tx<'a>(&'a mut Vec<Vec<u8>>);

impl RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0u8; len];
        let result = f(&mut packet);
        self.0.push(packet);
        result
    }
}

impl Device for QueueDevice {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Rx, Tx<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((Rx(packet), Tx(&mut self.tx)))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Tx<'_>> {
        Some(Tx(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}

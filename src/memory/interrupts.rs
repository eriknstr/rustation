/// The Playstation supports 10 interrupts
#[derive(Clone, Copy)]
pub enum Interrupt {
    /// Display in vertical blanking
    VBlank = 0,
}

#[derive(Clone,Copy)]
pub struct InterruptState {
    /// Interrupt status
    status: u16,
    /// Interrupt mask
    mask: u16,
}

impl InterruptState {

    pub fn new() -> InterruptState {
        InterruptState {
            status: 0,
            mask:   0,
        }
    }

    /// Return true if at least one interrupt is active and not masked
    pub fn active(self) -> bool {
        (self.status & self.mask) != 0
    }

    pub fn status(self) -> u16 {
        self.status
    }

    /// Acknowledge interrupts by writing 0 to the corresponding bit
    pub fn ack(&mut self, ack: u16) {
        self.status &= ack;
    }

    pub fn mask(self) -> u16 {
        self.mask
    }

    pub fn set_mask(&mut self, mask: u16) {
        self.mask = mask;
    }

    pub fn set_high(&mut self, which: Interrupt) {
        self.status |= 1 << (which as usize);
    }
}
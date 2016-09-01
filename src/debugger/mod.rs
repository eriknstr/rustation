use cpu::Cpu;

use tracer::Tracer;

/// Trait defining the debugger interface
pub trait Debugger {
    /// Signal a "break" which will put the emulator in debug mode at
    /// the next instruction
    fn trigger_break(&mut self);

    /// Called by the CPU when it's about to execute a new
    /// instruction. This function is called before *all* CPU
    /// instructions so it needs to be as fast as possible.
    fn pc_change<T: Tracer>(&mut self, cpu: &mut Cpu<T>);

    /// Called by the CPU when it's about to load a value from memory.
    fn memory_read<T: Tracer>(&mut self, cpu: &mut Cpu<T>, addr: u32);

    /// Called by the CPU when it's about to write a value to memory.
    fn memory_write<T: Tracer>(&mut self, cpu: &mut Cpu<T>, addr: u32);
}


/// Dummy debugger implementation that does nothing. Can be used when
/// debugging is disabled.
impl Debugger for () {
    fn trigger_break(&mut self) {
    }

    fn pc_change<T: Tracer>(&mut self, _: &mut Cpu<T>) {
    }

    fn memory_read<T: Tracer>(&mut self, _: &mut Cpu<T>, _: u32) {
    }

    fn memory_write<T: Tracer>(&mut self, _: &mut Cpu<T>, _: u32) {
    }
}

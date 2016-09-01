//! Interface used to log internal variables in order to generate
//! traces

/// Underlying type of every logged value. Since we use a `u32` we
/// only support variables up to 32bits for now.
pub type Value = u32;

pub trait Tracer {
    /// Called when a value change should be logged. A given
    /// `variable` should always have the same `size` in subsequent
    /// calls to `event`, otherwise the function is allowed to
    /// `panic!`.
    fn event(&mut self,
             date: u64,
             variable: &str,
             size: u8,
             value: Value);

    /// Return the list of variables handled by this tracer
    fn variables(&self) -> &[Variable];

    /// Return the full log of events
    fn log(&self) -> &[Event];

    /// Clear the log
    fn clear(&mut self);
}

/// Dummy implementation when we want to inhibit the tracing
impl Tracer for () {
    fn event(&mut self,
             _date: u64,
             _variable: &str,
             _size: u8,
             _value: Value) {
    }

    fn variables(&self) -> &[Variable] {
        &[]
    }

    fn log(&self) -> &[Event] {
        &[]
    }

    fn clear(&mut self) {
    }
}

/// Struct recording a single event: (date, id, value). `id` is the
/// index in the array returned by the `Tracer` `variables` method.
pub struct Event(pub u64, pub u32, pub Value);

#[derive(Clone)]
pub struct Variable {
    /// Name of the variable
    name: String,
    /// Size of the variable in bits.
    size: u8,
}

impl Variable {
    pub fn new(name: String, size: u8) -> Variable {
        Variable {
            name: name,
            size: size,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> u8 {
        self.size
    }
}

/// Trait implemented by visitors that are supposed to collect and
/// aggregate the various logs
pub trait Collector {
    type Error;

    /// Collect `tracer`'s log and clear it.
    fn collect<T: Tracer>(&mut self, tracer: &mut T) -> Self::Error;

    /// Used to dump a submodule, the submodule collection should
    /// happen in `f`
    fn submodule<F>(&mut self, name: &str, f: F) -> Self::Error
        where F: FnOnce(&mut Self) -> Self::Error;
}

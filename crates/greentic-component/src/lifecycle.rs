use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    pub init: bool,
    pub health: bool,
    pub shutdown: bool,
}

impl Lifecycle {
    pub fn is_noop(&self) -> bool {
        !(self.init || self.health || self.shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::Lifecycle;

    #[test]
    fn default_lifecycle_is_noop() {
        assert!(Lifecycle::default().is_noop());
    }

    #[test]
    fn any_enabled_hook_makes_lifecycle_non_noop() {
        assert!(
            !Lifecycle {
                init: true,
                ..Lifecycle::default()
            }
            .is_noop()
        );
        assert!(
            !Lifecycle {
                health: true,
                ..Lifecycle::default()
            }
            .is_noop()
        );
        assert!(
            !Lifecycle {
                shutdown: true,
                ..Lifecycle::default()
            }
            .is_noop()
        );
    }
}

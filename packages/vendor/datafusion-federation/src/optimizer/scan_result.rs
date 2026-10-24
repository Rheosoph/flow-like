use datafusion::common::tree_node::TreeNodeRecursion;

use crate::FederationProviderRef;

/// Used to track if all sources, including tableScan, plan inputs and
/// expressions, represents an un-ambiguous, none or a sole' [`crate::FederationProvider`].
pub enum ScanResult {
    None,
    Distinct(FederationProviderRef),
    Ambiguous,
}

impl ScanResult {
    pub fn merge(&mut self, other: Self) {
        match (&self, &other) {
            (_, ScanResult::None) => {}
            (ScanResult::None, _) => *self = other,
            (ScanResult::Ambiguous, _) | (_, ScanResult::Ambiguous) => {
                *self = ScanResult::Ambiguous
            }
            (ScanResult::Distinct(provider), ScanResult::Distinct(other_provider)) => {
                if provider != other_provider {
                    *self = ScanResult::Ambiguous
                }
            }
        }
    }

    pub fn add(&mut self, provider: Option<FederationProviderRef>) {
        self.merge(ScanResult::from(provider))
    }

    pub fn is_ambiguous(&self) -> bool {
        matches!(self, ScanResult::Ambiguous)
    }

    pub fn is_none(&self) -> bool {
        matches!(self, ScanResult::None)
    }
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    pub fn unwrap(self) -> Option<FederationProviderRef> {
        match self {
            ScanResult::None => None,
            ScanResult::Distinct(provider) => Some(provider),
            ScanResult::Ambiguous => panic!("called `ScanResult::unwrap()` on a `Ambiguous` value"),
        }
    }

    pub fn check_recursion(&self) -> TreeNodeRecursion {
        if self.is_ambiguous() {
            TreeNodeRecursion::Stop
        } else {
            TreeNodeRecursion::Continue
        }
    }
}

impl From<Option<FederationProviderRef>> for ScanResult {
    fn from(provider: Option<FederationProviderRef>) -> Self {
        match provider {
            Some(provider) => ScanResult::Distinct(provider),
            None => ScanResult::None,
        }
    }
}

impl PartialEq<Option<FederationProviderRef>> for ScanResult {
    fn eq(&self, other: &Option<FederationProviderRef>) -> bool {
        match (self, other) {
            (ScanResult::None, None) => true,
            (ScanResult::Distinct(provider), Some(other_provider)) => provider == other_provider,
            _ => false,
        }
    }
}

impl std::fmt::Debug for ScanResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ScanResult::None"),
            Self::Distinct(provider) => write!(f, "ScanResult::Distinct({})", provider.name()),
            Self::Ambiguous => write!(f, "ScanResult::Ambiguous"),
        }
    }
}

impl Clone for ScanResult {
    fn clone(&self) -> Self {
        match self {
            ScanResult::None => ScanResult::None,
            ScanResult::Distinct(provider) => ScanResult::Distinct(provider.clone()),
            ScanResult::Ambiguous => ScanResult::Ambiguous,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum CStandard {
    C99,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExtensionPolicy {
    Allow,
    Warn,
    Deny,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct CompilerConfiguration {
    standard:         CStandard,
    extension_policy: ExtensionPolicy,
}

impl CompilerConfiguration {
    pub(crate) const fn new(standard: CStandard, extension_policy: ExtensionPolicy) -> Self {
        Self {
            standard,
            extension_policy,
        }
    }

    pub(crate) const fn standard(self) -> CStandard {
        self.standard
    }

    pub(crate) const fn extension_policy(self) -> ExtensionPolicy {
        self.extension_policy
    }
}

impl Default for CompilerConfiguration {
    fn default() -> Self {
        Self::new(CStandard::C99, ExtensionPolicy::Allow)
    }
}

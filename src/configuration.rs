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

const _: (ExtensionPolicy, ExtensionPolicy) = (ExtensionPolicy::Warn, ExtensionPolicy::Deny);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct CompilerConfiguration {
    standard:                    CStandard,
    extension_policy:            ExtensionPolicy,
    repeated_specifier_warnings: bool,
    /// Seconds since the Unix epoch that `__DATE__` and `__TIME__` spell, in
    /// UTC, or `None` for the local time of translation.
    source_date_epoch:           Option<i64>,
}

impl CompilerConfiguration {
    pub(crate) const fn new(standard: CStandard, extension_policy: ExtensionPolicy) -> Self {
        Self {
            standard,
            extension_policy,
            repeated_specifier_warnings: true,
            source_date_epoch: None,
        }
    }

    pub(crate) const fn standard(self) -> CStandard {
        self.standard
    }

    pub(crate) const fn extension_policy(self) -> ExtensionPolicy {
        self.extension_policy
    }

    pub(crate) const fn repeated_specifier_warnings(self) -> bool {
        self.repeated_specifier_warnings
    }

    pub(crate) const fn with_repeated_specifier_warnings(mut self, enabled: bool) -> Self {
        self.repeated_specifier_warnings = enabled;
        self
    }

    pub(crate) const fn source_date_epoch(self) -> Option<i64> {
        self.source_date_epoch
    }

    pub(crate) const fn with_source_date_epoch(mut self, seconds: Option<i64>) -> Self {
        self.source_date_epoch = seconds;
        self
    }
}

impl Default for CompilerConfiguration {
    fn default() -> Self {
        Self::new(CStandard::C99, ExtensionPolicy::Allow)
    }
}

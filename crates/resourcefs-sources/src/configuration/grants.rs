use super::ConfigurationError;

/// Independent mutation authority. Every absent profile field maps to `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutationGrants {
    create: bool,
    update: bool,
    delete: bool,
}

impl MutationGrants {
    /// Constructs one explicit operation set.
    pub const fn new(create: bool, update: bool, delete: bool) -> Self {
        Self {
            create,
            update,
            delete,
        }
    }

    /// Whether creation is granted.
    pub const fn create(self) -> bool {
        self.create
    }

    /// Whether replacement or editing is granted.
    pub const fn update(self) -> bool {
        self.update
    }

    /// Whether deletion is granted.
    pub const fn delete(self) -> bool {
        self.delete
    }

    const fn is_subset_of(self, parent: Self) -> bool {
        (!self.create || parent.create)
            && (!self.update || parent.update)
            && (!self.delete || parent.delete)
    }
}

/// Mutation operations supported by one source kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationSupport(MutationGrants);

impl MutationSupport {
    /// A source that permits no mutation.
    pub const READ_ONLY: Self = Self(MutationGrants::new(false, false, false));
    /// GitHub's version-1 create/update authority; deletion is excluded.
    pub const GITHUB: Self = Self(MutationGrants::new(true, true, false));
    /// A source that supports independent create, update, and delete grants.
    pub const FULL: Self = Self(MutationGrants::new(true, true, true));

    /// Rejects operations that the source kind cannot implement.
    pub fn validate(self, grants: MutationGrants) -> Result<(), ConfigurationError> {
        if grants.is_subset_of(self.0) {
            Ok(())
        } else {
            Err(ConfigurationError::new(
                "mutation grants exceed the operations supported by this source kind",
            ))
        }
    }

    /// Rejects a child grant that exceeds either kind support or its parent grant.
    pub fn validate_nested(
        self,
        parent: MutationGrants,
        child: MutationGrants,
    ) -> Result<(), ConfigurationError> {
        self.validate(parent)?;
        self.validate(child)?;
        if child.is_subset_of(parent) {
            Ok(())
        } else {
            Err(ConfigurationError::new(
                "nested mutation grants must be a subset of source grants",
            ))
        }
    }
}

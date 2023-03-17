use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// See [`TargetInner`] for public fields reference.
#[derive(Clone, Debug)]
pub struct Target(Arc<TargetInner>);

#[derive(Debug)]
#[non_exhaustive]
pub struct TargetInner {
    pub kind: SmolStr,
    pub name: SmolStr,
    pub source_path: Utf8PathBuf,
    pub params: toml::Value,
}

impl Deref for Target {
    type Target = TargetInner;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl Target {
    pub const LIB: &'static str = "lib";

    pub fn new(
        kind: impl Into<SmolStr>,
        name: impl Into<SmolStr>,
        source_path: impl Into<Utf8PathBuf>,
        params: toml::Value,
    ) -> Self {
        assert!(params.is_table(), "params must be a TOML table");
        Self(Arc::new(TargetInner {
            kind: kind.into(),
            name: name.into(),
            source_path: source_path.into(),
            params,
        }))
    }

    pub fn without_params(
        kind: impl Into<SmolStr>,
        name: impl Into<SmolStr>,
        source_path: impl Into<Utf8PathBuf>,
    ) -> Self {
        Self::new(
            kind,
            name,
            source_path,
            toml::Value::Table(toml::Table::new()),
        )
    }

    pub fn try_from_structured_params(
        kind: impl Into<SmolStr>,
        name: impl Into<SmolStr>,
        source_path: impl Into<Utf8PathBuf>,
        params: impl Serialize,
    ) -> Result<Self> {
        let params = toml::Value::try_from(params)?;
        Ok(Self::new(kind, name, source_path, params))
    }

    pub fn is_lib(&self) -> bool {
        self.kind == Self::LIB
    }

    pub fn source_root(&self) -> &Utf8Path {
        self.source_path
            .parent()
            .expect("Source path is guaranteed to point to a file.")
    }

    pub fn props<'de, P>(&self) -> Result<P>
    where
        P: Default + Serialize + Deserialize<'de>,
    {
        let mut params = toml::Value::try_from(P::default())?;

        params.as_table_mut().unwrap().extend(
            self.params
                .as_table()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );

        let props = toml::Value::try_into(params)?;
        Ok(props)
    }
}

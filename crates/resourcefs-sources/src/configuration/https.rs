use url::Url;

use super::{
    ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    SecretReference, validate_configuration_id,
};

/// A validated HTTP credential-header definition with an unresolved secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialHeader {
    header: String,
    scheme: Option<String>,
    secret: SecretReference,
}

impl CredentialHeader {
    pub fn new(
        header: impl Into<String>,
        scheme: Option<String>,
        secret: SecretReference,
    ) -> Result<Self, ConfigurationError> {
        let header = header.into();
        if !is_http_token(&header) {
            return Err(ConfigurationError::new(
                "credential header name must use HTTP token syntax",
            ));
        }
        let normalized = header.to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "host"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "content-length"
                | "forwarded"
                | "via"
                | "x-forwarded-host"
                | "x-forwarded-port"
                | "x-forwarded-proto"
        ) {
            return Err(ConfigurationError::new(
                "credential header cannot control HTTP routing or framing",
            ));
        }
        if scheme.as_deref().is_some_and(|value| !is_http_token(value)) {
            return Err(ConfigurationError::new(
                "credential scheme must use non-empty HTTP token syntax",
            ));
        }
        Ok(Self {
            header,
            scheme,
            secret,
        })
    }
}

/// One candidate HTTPS base-prefix authority, validated by [`HttpsConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsOrigin {
    base_url: String,
    allow_private_network: bool,
    credential: Option<CredentialHeader>,
}

impl HttpsOrigin {
    pub fn new(
        base_url: impl Into<String>,
        allow_private_network: bool,
        credential: Option<CredentialHeader>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            allow_private_network,
            credential,
        }
    }
}

/// Validated static authority for the HTTPS Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsConfig {
    id: String,
    required: bool,
    origins: Vec<HttpsOrigin>,
}

impl HttpsConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        origins: Vec<HttpsOrigin>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        validate_collection_size("HTTPS origins", origins.len())?;

        let mut prefixes = Vec::with_capacity(origins.len());
        for origin in &origins {
            let url = validate_https_url(&origin.base_url)?;
            prefixes.push(HttpsPrefix::from_url(&url));
        }
        prefixes.sort_unstable();
        if prefixes.windows(2).any(|pair| pair[0].overlaps(&pair[1])) {
            return Err(ConfigurationError::new(
                "HTTPS base URL prefixes must be distinct and non-overlapping",
            ));
        }

        Ok(Self {
            id,
            required,
            origins,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HttpsPrefix {
    host: String,
    port: u16,
    components: Vec<String>,
}

impl HttpsPrefix {
    fn from_url(url: &Url) -> Self {
        let path = url.path().trim_end_matches('/').trim_start_matches('/');
        let components = if path.is_empty() {
            Vec::new()
        } else {
            path.split('/').map(str::to_owned).collect()
        };
        Self {
            host: url.host_str().unwrap_or_default().to_owned(),
            port: url.port_or_known_default().unwrap_or(443),
            components,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.host == other.host
            && self.port == other.port
            && (self.components.starts_with(&other.components)
                || other.components.starts_with(&self.components))
    }
}

pub(super) fn validate_https_url(input: &str) -> Result<Url, ConfigurationError> {
    let url = Url::parse(input)
        .map_err(|_| ConfigurationError::new("URL must be a valid absolute HTTPS URL"))?;
    if url.scheme() != "https"
        || url.cannot_be_a_base()
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_some_and(|host| host.contains('*'))
    {
        return Err(ConfigurationError::new(
            "URL must use HTTPS with a concrete host and no userinfo, query, or fragment",
        ));
    }
    Ok(url)
}

fn validate_collection_size(name: &str, size: usize) -> Result<(), ConfigurationError> {
    if size == 0 || size > MAX_CONFIGURATION_ENTRIES {
        return Err(ConfigurationError::new(format!(
            "{name} must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
        )));
    }
    Ok(())
}

fn is_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

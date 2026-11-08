//! OIDC（OpenID Connect）の認可コード + PKCE クライアント。
//!
//! discovery と ID token の署名・issuer・audience・期限・nonce 検証は
//! `openidconnect` crate に任せ、ここでは設定の解釈と業務に必要な
//! `subject` の取り出しだけを行う。トークンはログに出さない。

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::reqwest::redirect::Policy;
use openidconnect::reqwest::{Client, ClientBuilder};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse,
};

type OidcCoreClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SCOPES: &str = "openid email profile";

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub provider_label: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("OIDC の設定が不完全です: {0}")]
    Config(String),
    #[error("OIDC discovery に失敗しました: {0}")]
    Discovery(String),
    #[error("OIDC の認可 URL を組み立てられません: {0}")]
    Authorization(String),
    #[error("OIDC のトークン交換に失敗しました: {0}")]
    TokenExchange(String),
    #[error("ID token がありません")]
    MissingIdToken,
    #[error("ID token の検証に失敗しました: {0}")]
    InvalidIdToken(String),
    #[error("logout token の検証に失敗しました: {0}")]
    InvalidLogoutToken(String),
}

#[derive(Debug, Clone)]
pub struct OidcLogout {
    pub subject: Option<String>,
    pub sid: Option<String>,
    pub jti: String,
}

#[derive(Debug, serde::Deserialize)]
struct LogoutClaims {
    sub: Option<String>,
    sid: Option<String>,
    jti: Option<String>,
    nonce: Option<serde_json::Value>,
    events: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone)]
pub struct OidcIdentity {
    pub subject: String,
    /// IdP 側のセッション ID（back-channel logout の `sid`）。
    pub sid: Option<String>,
    /// `groups` claim。グループ→ロール同期に使う。
    pub groups: Vec<String>,
}

impl OidcConfig {
    /// 環境変数から設定する。OIDC_* が 1 つも無ければ `None`（パスワードログインのみ）。
    ///
    /// 一部だけ設定されている場合は起動時に失敗させる。SSO を有効にしたつもりで
    /// 無効のまま動く事故を防ぐ。
    pub fn from_env() -> Result<Option<Self>, OidcError> {
        let issuer = non_empty("OIDC_ISSUER");
        let client_id = non_empty("OIDC_CLIENT_ID");
        let client_secret = non_empty("OIDC_CLIENT_SECRET");
        let redirect_uri = non_empty("OIDC_REDIRECT_URI");

        if issuer.is_none()
            && client_id.is_none()
            && client_secret.is_none()
            && redirect_uri.is_none()
        {
            return Ok(None);
        }

        let issuer = issuer.ok_or_else(|| OidcError::Config("OIDC_ISSUER がありません".into()))?;
        let client_id =
            client_id.ok_or_else(|| OidcError::Config("OIDC_CLIENT_ID がありません".into()))?;
        let client_secret = client_secret
            .ok_or_else(|| OidcError::Config("OIDC_CLIENT_SECRET がありません".into()))?;
        let redirect_uri = match redirect_uri {
            Some(v) => v,
            None => {
                let base = non_empty("PUBLIC_BASE_URL").ok_or_else(|| {
                    OidcError::Config("OIDC_REDIRECT_URI も PUBLIC_BASE_URL もありません".into())
                })?;
                format!("{}/api/v1/auth/oidc/callback", base.trim_end_matches('/'))
            }
        };

        // `openid` は openidconnect が認可 URL に自動で付ける。重複させない。
        let scopes: Vec<String> = non_empty("OIDC_SCOPES")
            .unwrap_or_else(|| DEFAULT_SCOPES.to_owned())
            .split_whitespace()
            .filter(|s| *s != "openid")
            .map(str::to_owned)
            .collect();

        Ok(Some(Self {
            issuer,
            client_id,
            client_secret,
            redirect_uri,
            provider_label: non_empty("OIDC_PROVIDER_LABEL").unwrap_or_else(|| "SSO".to_owned()),
            scopes,
        }))
    }
}

fn non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// discovery の結果をキャッシュする。IdP 停止中でも起動でき、
/// 復旧後の最初のログインで再 discovery する。
pub struct OidcRuntime {
    config: OidcConfig,
    client: Mutex<Option<Arc<OidcClient>>>,
    logout_jti: Mutex<std::collections::HashMap<String, i64>>,
}

impl OidcRuntime {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            config,
            client: Mutex::new(None),
            logout_jti: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// logout token の `jti` を記録する。既知（リプレイ）なら `false`。
    pub async fn remember_logout_jti(&self, jti: &str) -> bool {
        let now = chrono::Utc::now().timestamp();
        let mut seen = self.logout_jti.lock().await;
        seen.retain(|_, expires| *expires >= now);
        if seen.contains_key(jti) {
            return false;
        }
        seen.insert(jti.to_owned(), now + 600);
        true
    }

    pub fn config(&self) -> &OidcConfig {
        &self.config
    }

    pub async fn client(&self) -> Result<Arc<OidcClient>, OidcError> {
        let mut cached = self.client.lock().await;
        if let Some(client) = cached.as_ref() {
            return Ok(client.clone());
        }
        let client = Arc::new(OidcClient::discover(&self.config).await?);
        *cached = Some(client.clone());
        Ok(client)
    }

    /// トークン交換が失敗したとき、次の要求で discovery からやり直す。
    pub async fn invalidate(&self) {
        *self.client.lock().await = None;
    }
}

pub struct OidcClient {
    http: Client,
    metadata: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_uri: RedirectUrl,
    scopes: Vec<Scope>,
}

impl OidcClient {
    async fn discover(config: &OidcConfig) -> Result<Self, OidcError> {
        let http = ClientBuilder::new()
            .redirect(Policy::none())
            .timeout(DISCOVERY_TIMEOUT)
            .build()
            .map_err(|e| OidcError::Discovery(e.to_string()))?;

        let issuer = IssuerUrl::new(config.issuer.clone())
            .map_err(|e| OidcError::Config(format!("OIDC_ISSUER が URL ではありません: {e}")))?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?;

        Ok(Self {
            http,
            metadata,
            client_id: ClientId::new(config.client_id.clone()),
            client_secret: ClientSecret::new(config.client_secret.clone()),
            redirect_uri: RedirectUrl::new(config.redirect_uri.clone())
                .map_err(|e| OidcError::Config(format!("OIDC_REDIRECT_URI が不正です: {e}")))?,
            scopes: config.scopes.iter().cloned().map(Scope::new).collect(),
        })
    }

    fn core_client(&self) -> OidcCoreClient {
        CoreClient::from_provider_metadata(
            self.metadata.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect_uri.clone())
    }

    pub fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        code_verifier: &str,
    ) -> Result<String, OidcError> {
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            code_verifier.to_owned(),
        ));
        let state = state.to_owned();
        let nonce = nonce.to_owned();
        let (url, _csrf, _nonce) = self
            .core_client()
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            .add_scopes(self.scopes.clone())
            .set_pkce_challenge(challenge)
            .url();
        Ok(url.to_string())
    }

    /// back-channel logout の logout token を検証する。
    pub fn verify_logout_token(&self, token: &str) -> Result<OidcLogout, OidcError> {
        let jwks_value = serde_json::to_value(self.metadata.jwks())
            .map_err(|e| OidcError::InvalidLogoutToken(e.to_string()))?;
        let jwks: JwkSet = serde_json::from_value(jwks_value)
            .map_err(|e| OidcError::InvalidLogoutToken(e.to_string()))?;
        let header =
            decode_header(token).map_err(|e| OidcError::InvalidLogoutToken(e.to_string()))?;
        if !matches!(
            header.alg,
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::ES256
                | Algorithm::ES384
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512
        ) {
            return Err(OidcError::InvalidLogoutToken(
                "署名アルゴリズムが許可されていません".into(),
            ));
        }
        let kid = header
            .kid
            .ok_or_else(|| OidcError::InvalidLogoutToken("kid がありません".into()))?;
        let jwk = jwks
            .find(&kid)
            .ok_or_else(|| OidcError::InvalidLogoutToken("未知の kid です".into()))?;
        let key =
            DecodingKey::from_jwk(jwk).map_err(|e| OidcError::InvalidLogoutToken(e.to_string()))?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.metadata.issuer().as_str()]);
        validation.set_audience(&[self.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.leeway = 30;

        let data = decode::<LogoutClaims>(token, &key, &validation)
            .map_err(|e| OidcError::InvalidLogoutToken(e.to_string()))?;
        let claims = data.claims;

        if claims.nonce.is_some() {
            return Err(OidcError::InvalidLogoutToken(
                "nonce を含めないでください".into(),
            ));
        }
        let backchannel = claims
            .events
            .as_ref()
            .map(|events| events.contains_key("http://schemas.openid.net/event/backchannel-logout"))
            .unwrap_or(false);
        if !backchannel {
            return Err(OidcError::InvalidLogoutToken(
                "back-channel logout event がありません".into(),
            ));
        }
        if claims.sub.is_none() && claims.sid.is_none() {
            return Err(OidcError::InvalidLogoutToken(
                "sub と sid の両方がありません".into(),
            ));
        }
        let jti = claims
            .jti
            .ok_or_else(|| OidcError::InvalidLogoutToken("jti がありません".into()))?;

        Ok(OidcLogout {
            subject: claims.sub,
            sid: claims.sid,
            jti,
        })
    }

    pub async fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
        nonce: &str,
    ) -> Result<OidcIdentity, OidcError> {
        let client = self.core_client();
        let response = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|e| OidcError::TokenExchange(e.to_string()))?
            .set_pkce_verifier(PkceCodeVerifier::new(code_verifier.to_owned()))
            .request_async(&self.http)
            .await
            .map_err(|e| OidcError::TokenExchange(e.to_string()))?;

        let id_token = response.id_token().ok_or(OidcError::MissingIdToken)?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &Nonce::new(nonce.to_owned()))
            .map_err(|e| OidcError::InvalidIdToken(e.to_string()))?;

        let extra = jwt_payload(id_token.to_string().as_str());
        let sid = extra
            .as_ref()
            .and_then(|value| value.get("sid"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let groups = extra
            .as_ref()
            .and_then(|value| value.get("groups"))
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        Ok(OidcIdentity {
            subject: claims.subject().as_str().to_owned(),
            sid,
            groups,
        })
    }
}

fn jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// RFC 7636 の S256 challenge。認可 URL と保存する verifier を一致させる。
pub fn pkce_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // RFC 7636 Appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}

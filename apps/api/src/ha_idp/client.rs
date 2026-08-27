//! Cliente real del proveedor Home Assistant: canje del código por HTTP y lectura de la
//! identidad por WebSocket.
//!
//! **Por qué WebSocket para la identidad**: HA no expone `auth/current_user` por REST. El
//! `access_token` del canje solo sirve para abrir el WebSocket y preguntar «¿quién soy?».
//!
//! Nada de este módulo se testea en unitario: sus partes puras (URLs, códecs) viven en
//! `super`, y lo que queda es I/O contra un Home Assistant real — que se prueba en el smoke en
//! vivo, no con un doble. En los tests de integración se sustituye entero por `FakeHaIdp` a
//! través del trait `HaIdp`.
//!
//! **Jamás se registra un token** (ni de acceso, ni de refresco, ni el código): a `debug` solo
//! van longitudes. Lo que sí se registra a `info` es el `ha_auth_provider` que devuelve HA —
//! dice con qué proveedor entró la persona y es la pista que hace falta cuando alguien reporta
//! «entré con X y no me reconoce».

use super::{ws_url_from_base, HaIdentity, HaIdpError, HaTokens};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Tope de la respuesta del canje. El cuerpo real son ~400 bytes; 64 KiB es holgura pura y
/// evita que un HA comprometido (o un proxy que devuelve una página de error enorme) nos haga
/// asignar memoria sin límite.
const MAX_TOKEN_BODY: usize = 64 * 1024;

/// Presupuesto de tramas antes de rendirse esperando la respuesta que toca. HA intercala
/// eventos y pings; cinco tramas cubren de sobra el ruido de un handshake sano.
const FRAME_BUDGET: usize = 5;

/// Tope del diálogo WebSocket **completo** (conexión + auth + `auth/current_user`).
const WS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub struct HttpHaIdp {
    base_url: String,
    http: reqwest::Client,
}

impl HttpHaIdp {
    /// `base_url` es el origen público de Home Assistant, ya validado al arrancar.
    ///
    /// `redirect(none)`: un 302 desde el endpoint de token no es un flujo legítimo, y seguirlo
    /// mandaría el `code` a donde diga un tercero. Los timeouts son cortos a propósito: esto
    /// corre dentro de una petición del navegador, y una espera larga se ve como una app colgada.
    pub fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("FutureFin/", env!("CARGO_PKG_VERSION")))
            .build()
            // Solo falla si el runtime TLS no se puede inicializar: no hay recuperación posible
            // ni configuración del operador que lo arregle.
            .expect("construir el cliente HTTP de Home Assistant");
        Self { base_url, http }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait::async_trait]
impl super::HaIdp for HttpHaIdp {
    async fn exchange_code(&self, code: &str, client_id: &str) -> Result<HaTokens, HaIdpError> {
        // `client_id` va tal cual llegó del `/start`: HA indexa su almacén de códigos por la
        // cadena CRUDA, así que recalcularlo aquí (con las cabeceras de OTRA petición) sería la
        // forma más silenciosa de romper el flujo.
        let resp = self
            .http
            .post(self.endpoint("/auth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", client_id),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "home assistant: fallo de transporte al canjear el código");
                HaIdpError::Transport
            })?;

        let status = resp.status();
        let body = resp.bytes().await.map_err(|e| {
            tracing::warn!(error = %e, "home assistant: no se pudo leer la respuesta del canje");
            HaIdpError::Transport
        })?;
        if !status.is_success() {
            tracing::warn!(%status, "home assistant rechazó el canje del código");
            return Err(HaIdpError::Exchange);
        }
        if body.len() > MAX_TOKEN_BODY {
            tracing::warn!(bytes = body.len(), "home assistant: respuesta de token desmesurada");
            return Err(HaIdpError::Exchange);
        }
        let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
            tracing::warn!(error = %e, "home assistant: la respuesta del canje no es JSON");
            HaIdpError::Exchange
        })?;
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                tracing::warn!("home assistant: la respuesta del canje no trae access_token");
                HaIdpError::Exchange
            })?
            .to_string();
        let refresh_token = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if let Some(provider) = json.get("ha_auth_provider").and_then(|v| v.as_str()) {
            tracing::info!(ha_auth_provider = provider, "home assistant: código canjeado");
        } else {
            tracing::info!("home assistant: código canjeado");
        }
        tracing::debug!(
            access_token_len = access_token.len(),
            has_refresh = refresh_token.is_some(),
            "home assistant: tokens recibidos"
        );
        Ok(HaTokens {
            access_token,
            refresh_token,
        })
    }

    async fn identity(&self, access_token: &str) -> Result<HaIdentity, HaIdpError> {
        // UN solo timeout para el diálogo entero: acotar cada lectura por separado deja abierta
        // la suma (un HA que responde despacio a cada paso mantendría la petición viva minutos).
        match tokio::time::timeout(WS_TIMEOUT, self.identity_dialogue(access_token)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("home assistant: el WebSocket de identidad agotó el tiempo");
                Err(HaIdpError::Transport)
            }
        }
    }

    async fn revoke(&self, refresh_token: &str) {
        // HA responde 200 haga lo que haga (no distingue «revocado» de «no existía»), así que
        // no hay nada que comprobar: esto es higiene, no una puerta.
        let sent = self
            .http
            .post(self.endpoint("/auth/revoke"))
            .form(&[("token", refresh_token)])
            .send()
            .await;
        match sent {
            Ok(_) => tracing::debug!("home assistant: refresh token revocado"),
            Err(e) => tracing::warn!(
                error = %e,
                "no se pudo revocar el refresh token de Home Assistant; si sobra, se borra a mano \
                 en Home Assistant → Perfil → Seguridad → Tokens de actualización"
            ),
        }
    }
}

impl HttpHaIdp {
    /// El diálogo completo: `auth_required` → `auth` → `auth_ok` → `auth/current_user` →
    /// `result`. Cualquier sorpresa de protocolo es `Identity`; los fallos de red, `Transport`.
    async fn identity_dialogue(&self, access_token: &str) -> Result<HaIdentity, HaIdpError> {
        let url = ws_url_from_base(&self.base_url);
        let config = WebSocketConfig::default()
            .max_message_size(Some(256 * 1024))
            .max_frame_size(Some(64 * 1024));
        let (mut ws, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), false)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "home assistant: no se pudo abrir el WebSocket");
                HaIdpError::Transport
            })?;

        read_frame_matching(&mut ws, |v| v["type"] == "auth_required").await?;
        send_json(
            &mut ws,
            &serde_json::json!({"type": "auth", "access_token": access_token}),
        )
        .await?;

        let auth = read_frame_matching(&mut ws, |v| {
            v["type"] == "auth_ok" || v["type"] == "auth_invalid"
        })
        .await?;
        if auth["type"] != "auth_ok" {
            tracing::warn!("home assistant rechazó el access_token del WebSocket");
            return Err(HaIdpError::Identity);
        }

        send_json(
            &mut ws,
            &serde_json::json!({"id": 1, "type": "auth/current_user"}),
        )
        .await?;
        let result = read_frame_matching(&mut ws, |v| v["id"] == 1 && v["type"] == "result").await?;
        // Cerrar explícitamente: sin esto la conexión queda a merced del timeout de HA y su log
        // se llena de desconexiones sucias por cada login.
        let _ = ws.close(None).await;

        if result["success"] != serde_json::Value::Bool(true) {
            tracing::warn!("home assistant: auth/current_user devolvió success=false");
            return Err(HaIdpError::Identity);
        }
        let raw_id = result["result"]["id"].as_str().ok_or_else(|| {
            tracing::warn!("home assistant: auth/current_user sin id de usuario");
            HaIdpError::Identity
        })?;
        // `uuid4().hex` — 32 hexadecimales SIN guiones. `parse_str` acepta esa forma y produce
        // el mismo UUID que la canónica de `X-Remote-User-Id`: es la paridad entre los dos
        // caminos de entrada (ver la cabecera de `super`).
        let external_user_id = Uuid::parse_str(raw_id).map_err(|_| {
            tracing::warn!("home assistant: el id de usuario no es un UUID");
            HaIdpError::Identity
        })?;
        let name = result["result"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(HaIdentity {
            external_user_id,
            name,
        })
    }
}

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn send_json(ws: &mut Ws, value: &serde_json::Value) -> Result<(), HaIdpError> {
    ws.send(Message::text(value.to_string()))
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "home assistant: fallo escribiendo en el WebSocket");
            HaIdpError::Transport
        })
}

/// Lee tramas hasta encontrar la que cumple `matches`, saltando ruido (eventos, pings,
/// binarios) con un presupuesto acotado. Sin presupuesto, un HA que emite eventos sin parar
/// mantendría el bucle vivo hasta el timeout global sin llegar nunca a nada.
async fn read_frame_matching(
    ws: &mut Ws,
    matches: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value, HaIdpError> {
    for _ in 0..FRAME_BUDGET {
        let frame = match ws.next().await {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                tracing::warn!(error = %e, "home assistant: fallo leyendo del WebSocket");
                return Err(HaIdpError::Transport);
            }
            None => {
                tracing::warn!("home assistant cerró el WebSocket antes de responder");
                return Err(HaIdpError::Identity);
            }
        };
        let text = match frame {
            Message::Text(t) => t,
            // Ping/Pong los gestiona tungstenite solo; Binary y Frame no existen en este
            // protocolo. Un Close es el fin del diálogo.
            Message::Close(_) => {
                tracing::warn!("home assistant cerró el WebSocket a mitad del diálogo");
                return Err(HaIdpError::Identity);
            }
            _ => continue,
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text.as_str()) else {
            continue;
        };
        if matches(&value) {
            return Ok(value);
        }
    }
    tracing::warn!("home assistant: no llegó la trama esperada dentro del presupuesto");
    Err(HaIdpError::Identity)
}

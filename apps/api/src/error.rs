use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use tracing::error;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    /// Clase HTTP del error (`bad_request`, `conflict`…). Contrato publicado desde 1.0.0.
    pub error: ErrorCode,
    /// Código ESTABLE y granular, pensado para que un cliente decida qué enseñar sin leer
    /// `message`. Sale del prefijo `snake_code:` del mensaje cuando lo hay
    /// (`username_taken: …` → `username_taken`); si no, cae a la clase HTTP (`bad_request`).
    /// La SPA lo traduce con `apps/web/src/lib/errorMessages.ts` — un código sin entrada en ese
    /// catálogo hace fallar `error_codes_have_spanish_copy`.
    pub code: String,
    /// Detalle técnico, en inglés y para desarrolladores. La SPA no lo enseña como frase
    /// principal: lo pliega bajo «Detalles técnicos».
    pub message: String,
}

impl ErrorBody {
    pub fn from_api_error(e: &ApiError) -> Self {
        let message = e.sanitised_message();
        let kind = e.code();
        ErrorBody {
            code: derive_error_code(kind, &message),
            error: kind,
            message,
        }
    }
}

/// Extrae el código estable del prefijo `snake_code: ` del mensaje. Sin prefijo válido, el código
/// es la clase HTTP. Deliberadamente estrecho —solo `[a-z][a-z0-9_]*` de 3 a 64 caracteres— para
/// que un mensaje corriente con dos puntos («Expected JSON: …», «Cena, entrada») no invente un
/// código: un código inventado es peor que ninguno, porque el catálogo del front no lo tendrá y
/// el usuario verá el fallback genérico creyendo que hay una traducción.
fn derive_error_code(kind: ErrorCode, message: &str) -> String {
    if let Some((prefix, _)) = message.split_once(": ") {
        let ok = (3..=64).contains(&prefix.len())
            && prefix.starts_with(|c: char| c.is_ascii_lowercase())
            && prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if ok {
            return prefix.to_string();
        }
    }
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "internal".into())
}

#[derive(Debug, Clone, Copy, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unprocessable,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    /// Petición bien formada pero rechazada por una regla de negocio (422). El mensaje sigue el
    /// estilo `snake_code: descripción` del resto de validaciones del módulo.
    #[error("{0}")]
    Unprocessable(String),
    #[error("unauthorized")]
    Unauthorized,
    /// 401 que SÍ propaga mensaje al wire, con prefijo `snake_code:`. El `Unauthorized` pelado es
    /// deliberadamente mudo (no delata qué credencial falló); existe este hermano para el caso
    /// contrario: cuando el 401 describe una situación de la CUENTA que el usuario necesita
    /// entender para salir de ella — «esta cuenta entra por el proxy y no tiene contraseña» —, y
    /// callarlo lo dejaría reintentando una contraseña que no existe.
    #[error("{0}")]
    UnauthorizedWith(String),
    #[error("forbidden")]
    Forbidden,
    /// 403 que SÍ propaga mensaje al wire, con prefijo `snake_code:`. El `Forbidden` pelado dice
    /// solo «forbidden» y su código es la clase HTTP, que basta cuando el motivo es el ROL (la
    /// SPA ya sabe qué hacer con un 403 de owner-only). Existe este hermano para el 403 que
    /// describe una regla de FILA — «esto es de otro miembro del hogar» (D21, 5.0.0) —, donde
    /// el cliente necesita distinguirlo de un permiso insuficiente para enseñar la frase
    /// correcta en vez de mandar al usuario a pedir que le suban el rol.
    #[error("{0}")]
    ForbiddenWith(String),
    #[error("resource conflict")]
    Conflict,
    /// 409 que SÍ propaga mensaje al wire, con prefijo `snake_code:`. El `Conflict` pelado nace
    /// del mapeo automático de SQLSTATE 23505 y no sabe QUÉ colisionó; cuando el handler sí lo
    /// sabe (un username ya registrado, un nombre de categoría repetido) debe decirlo: sin esto
    /// la SPA solo podía enseñar «resource conflict».
    #[error("{0}")]
    ConflictWith(String),
    #[error("database error")]
    Db(sqlx::Error),
    #[error("service unavailable")]
    Unavailable,
    #[error("not found")]
    NotFound,
    /// 404 que SÍ propaga mensaje al wire. Existe para las operaciones de lote, donde un «not
    /// found» pelado obligaría al llamante a buscar a ciegas cuál de los N ids falló — justo el
    /// trabajo de reconciliación que un lote todo-o-nada viene a evitar. El mensaje solo nombra
    /// ids que el llamante ya envió: no revela nada que no supiera.
    #[error("{0}")]
    NotFoundWith(String),
}

/// Mapea automáticamente los SQLSTATE más comunes a respuestas HTTP coherentes, evitando que
/// cada handler tenga que distinguir entre violación de unique, FK rota y otros errores. Códigos
/// menos comunes caen al `Db(_)` genérico → 500.
impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = err {
            match db.code().as_deref() {
                Some("23505") => return ApiError::Conflict,
                Some("23503") => {
                    return ApiError::BadRequest("referenced record missing".into());
                }
                // 22003 = numeric_value_out_of_range. Las columnas de dinero son NUMERIC(18,4)
                // (14 dígitos enteros), así que un importe absurdo las desborda en el INSERT.
                // Sin este brazo caía a `Db(_)` → 500 «internal error» pelado: el ÚNICO error de
                // toda la superficie que un cliente no puede clasificar, y justo el que dispara
                // las políticas de retry-on-5xx contra una entrada que nunca va a ser válida.
                // Un agente desatendido entraba en bucle. Es 400: la entrada es el problema.
                Some("22003") => {
                    return ApiError::BadRequest(
                        "amount_out_of_range: el importe excede el máximo que admite la base \
                         (hasta 14 dígitos enteros y 4 decimales)"
                            .into(),
                    );
                }
                _ => {}
            }
        }
        ApiError::Db(err)
    }
}

impl ApiError {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::Unauthorized | ApiError::UnauthorizedWith(_) => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden | ApiError::ForbiddenWith(_) => StatusCode::FORBIDDEN,
            ApiError::NotFound | ApiError::NotFoundWith(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict | ApiError::ConflictWith(_) => StatusCode::CONFLICT,
            ApiError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn sanitised_message(&self) -> String {
        match self {
            ApiError::BadRequest(s) => s.clone(),
            ApiError::Unprocessable(s) => s.clone(),
            ApiError::Unauthorized => "authentication required".into(),
            ApiError::UnauthorizedWith(s) => s.clone(),
            ApiError::Forbidden => "forbidden".into(),
            ApiError::ForbiddenWith(s) => s.clone(),
            ApiError::NotFound => "not found".into(),
            ApiError::NotFoundWith(s) => s.clone(),
            ApiError::Conflict => "resource conflict".into(),
            ApiError::ConflictWith(s) => s.clone(),
            ApiError::Unavailable => "dependency unavailable".into(),
            ApiError::Db(err) => {
                error!(?err, "database error");
                "internal error".into()
            }
        }
    }

    pub(crate) fn code(&self) -> ErrorCode {
        match self {
            ApiError::BadRequest(_) => ErrorCode::BadRequest,
            ApiError::Unprocessable(_) => ErrorCode::Unprocessable,
            ApiError::Unauthorized | ApiError::UnauthorizedWith(_) => ErrorCode::Unauthorized,
            ApiError::Forbidden | ApiError::ForbiddenWith(_) => ErrorCode::Forbidden,
            ApiError::NotFound | ApiError::NotFoundWith(_) => ErrorCode::NotFound,
            ApiError::Conflict | ApiError::ConflictWith(_) => ErrorCode::Conflict,
            ApiError::Unavailable => ErrorCode::Unavailable,
            ApiError::Db(_) => ErrorCode::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        (status, Json(ErrorBody::from_api_error(&self))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_comes_from_the_snake_prefix_when_there_is_one() {
        assert_eq!(
            derive_error_code(ErrorCode::BadRequest, "username_taken: ya existe"),
            "username_taken"
        );
        assert_eq!(
            derive_error_code(ErrorCode::BadRequest, "csv_preset_unrecognized: could not detect"),
            "csv_preset_unrecognized"
        );
        // Dígitos y guiones bajos son válidos dentro del código.
        assert_eq!(
            derive_error_code(ErrorCode::Unprocessable, "swr_pct_out_of_range_0_4: fuera"),
            "swr_pct_out_of_range_0_4"
        );
    }

    #[test]
    fn code_falls_back_to_the_http_class_without_a_valid_prefix() {
        // Sin dos puntos.
        assert_eq!(
            derive_error_code(ErrorCode::BadRequest, "amount must be >= 0"),
            "bad_request"
        );
        // Con dos puntos pero el prefijo lleva espacios: no es un código.
        assert_eq!(
            derive_error_code(ErrorCode::Internal, "compound marker task panic: boom"),
            "internal"
        );
        // Mayúsculas: tampoco.
        assert_eq!(
            derive_error_code(ErrorCode::BadRequest, "Expected JSON: nope"),
            "bad_request"
        );
        // Demasiado corto para ser un código con sentido.
        assert_eq!(derive_error_code(ErrorCode::NotFound, "ab: x"), "not_found");
        // Los errores sin mensaje propio caen a su clase.
        assert_eq!(
            derive_error_code(ErrorCode::Unauthorized, "authentication required"),
            "unauthorized"
        );
    }

    #[test]
    fn body_carries_class_code_and_message() {
        let b = ErrorBody::from_api_error(&ApiError::ConflictWith(
            "username_taken: ese nombre ya está registrado".into(),
        ));
        assert_eq!(b.code, "username_taken");
        assert!(matches!(b.error, ErrorCode::Conflict));
        assert!(b.message.contains("username_taken"));

        // El 409 automático del SQLSTATE 23505 sigue sin código granular: es lo correcto,
        // no sabe qué colisionó.
        let generic = ErrorBody::from_api_error(&ApiError::Conflict);
        assert_eq!(generic.code, "conflict");
    }
}

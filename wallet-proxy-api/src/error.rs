use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error_message: String,
    pub error: ErrorCode,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum ErrorCode {
    /// Internal server error. E.g. if the server could not communicate with the node
    Internal,
    /// The given request is invalid
    InvalidRequest,
    /// Requested resource could not be found
    NotFound,
}

impl ErrorCode {
    fn get_code(self) -> u8 {
        match self {
            ErrorCode::Internal => 0,
            ErrorCode::InvalidRequest => 1,
            ErrorCode::NotFound => 2,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Internal),
            1 => Some(Self::InvalidRequest),
            2 => Some(Self::NotFound),
            _ => None,
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let code = self.get_code();

        code.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let code = u8::deserialize(deserializer)?;

        Self::from_code(code)
            .ok_or_else(|| de::Error::custom(format!("invalid error code: {}", code)))
    }
}

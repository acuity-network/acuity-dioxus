use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct IndexerSpan {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Serialize)]
pub struct IndexerRequest<T> {
    pub id: u64,
    #[serde(rename = "type")]
    pub message_type: &'static str,
    #[serde(flatten)]
    pub payload: T,
}

#[derive(Clone, Serialize, Default)]
pub struct EmptyPayload {}

#[derive(Clone, Serialize)]
pub struct GetEventsPayload {
    pub key: IndexerKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<EventCursor>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum IndexerKey {
    Custom(IndexerCustomKey),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IndexerCustomKey {
    pub name: String,
    pub kind: String,
    pub value: IndexerScalarValue,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum IndexerScalarValue {
    String(String),
    U32(u32),
    Bool(bool),
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventCursor {
    pub block_number: u32,
    pub event_index: u16,
}

#[derive(Clone, Deserialize)]
pub struct IndexerEnvelope {
    pub id: Option<u64>,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Clone, Deserialize)]
pub struct IndexerErrorPayload {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexerSubscriptionTerminatedPayload {
    pub reason: String,
    pub message: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerEventsData {
    #[serde(default)]
    pub decoded_events: Vec<IndexerDecodedEvent>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerDecodedEvent {
    pub event: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerStoredEvent {
    pub pallet_name: String,
    pub event_name: String,
    pub fields: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_status_request_with_numeric_id() {
        let request = IndexerRequest {
            id: 1,
            message_type: "Status",
            payload: EmptyPayload::default(),
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["id"], 1);
        assert_eq!(json["type"], "Status");
        assert_eq!(json.as_object().unwrap().len(), 2);
    }

    #[test]
    fn serializes_get_events_request_with_key_and_optional_fields() {
        let request = IndexerRequest {
            id: 3,
            message_type: "GetEvents",
            payload: GetEventsPayload {
                key: IndexerKey::Custom(IndexerCustomKey {
                    name: "item_id".to_string(),
                    kind: "bytes32".to_string(),
                    value: IndexerScalarValue::String("0x12".to_string()),
                }),
                limit: Some(25),
                before: Some(EventCursor {
                    block_number: 50,
                    event_index: 3,
                }),
            },
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["id"], 3);
        assert_eq!(json["type"], "GetEvents");
        assert_eq!(json["key"]["type"], "Custom");
        assert_eq!(json["key"]["value"]["name"], "item_id");
        assert_eq!(json["key"]["value"]["kind"], "bytes32");
        assert_eq!(json["key"]["value"]["value"], "0x12");
        assert_eq!(json["limit"], 25);
        assert_eq!(json["before"]["blockNumber"], 50);
        assert_eq!(json["before"]["eventIndex"], 3);
    }

    #[test]
    fn deserializes_error_envelope() {
        let envelope = serde_json::from_str::<IndexerEnvelope>(
            r#"{"id":9,"type":"error","data":{"code":"invalid_request","message":"missing field `id`"}}"#,
        )
        .unwrap();

        let error = serde_json::from_value::<IndexerErrorPayload>(envelope.data.unwrap()).unwrap();

        assert_eq!(envelope.id, Some(9));
        assert_eq!(envelope.message_type, "error");
        assert_eq!(error.code, "invalid_request");
        assert_eq!(error.message, "missing field `id`");
    }

    #[test]
    fn deserializes_bool_scalar_value() {
        let key = serde_json::from_str::<IndexerKey>(
            r#"{"type":"Custom","value":{"name":"published","kind":"bool","value":true}}"#,
        )
        .unwrap();

        assert_eq!(
            key,
            IndexerKey::Custom(IndexerCustomKey {
                name: "published".to_string(),
                kind: "bool".to_string(),
                value: IndexerScalarValue::Bool(true),
            })
        );
    }
}

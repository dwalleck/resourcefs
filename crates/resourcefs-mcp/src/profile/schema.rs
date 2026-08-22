use std::sync::LazyLock;

use schemars::generate::SchemaSettings;

use super::ProfileDocument;

static PROFILE_SCHEMA_JSON: LazyLock<String> = LazyLock::new(|| {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.into_root_schema_for::<ProfileDocument>();
    let mut value = serde_json::to_value(schema).expect("Profile schema must serialize");
    let object = value
        .as_object_mut()
        .expect("Profile schema root must be an object");
    object.insert(
        "$id".to_owned(),
        serde_json::Value::String(
            "https://resourcefs.dev/schema/server-profile-v1.json".to_owned(),
        ),
    );
    remove_null_defaults(&mut value);
    serde_json::to_string_pretty(&value).expect("Profile schema must serialize as JSON")
});

/// Returns deterministic pretty JSON Schema 2020-12 for Server Profile v1.
pub fn profile_schema_json() -> &'static str {
    &PROFILE_SCHEMA_JSON
}

fn remove_null_defaults(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            if object
                .get("default")
                .is_some_and(serde_json::Value::is_null)
            {
                object.remove("default");
            }
            for nested in object.values_mut() {
                remove_null_defaults(nested);
            }
        }
        serde_json::Value::Array(array) => {
            for nested in array {
                remove_null_defaults(nested);
            }
        }
        _ => {}
    }
}

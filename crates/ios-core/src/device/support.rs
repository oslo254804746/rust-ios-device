fn plist_value_to_string(value: &plist::Value, field: &str) -> Result<String, CoreError> {
    value.as_string().map(ToOwned::to_owned).ok_or_else(|| {
        CoreError::Protocol(format!("{field} expected string value, got {:?}", value))
    })
}

fn plist_value_to_string_vec(value: &plist::Value, field: &str) -> Result<Vec<String>, CoreError> {
    let values = value.as_array().ok_or_else(|| {
        CoreError::Protocol(format!(
            "{field} expected string array value, got {:?}",
            value
        ))
    })?;

    values
        .iter()
        .map(|item| {
            item.as_string().map(ToOwned::to_owned).ok_or_else(|| {
                CoreError::Protocol(format!("{field} expected string entries, got {:?}", item))
            })
        })
        .collect()
}

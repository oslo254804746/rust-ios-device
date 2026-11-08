//! Finite typed GetValue projection, with validation/admission before cloning.

use super::super::{send_lockdown, GetValueRequest, LockdownError, LockdownResponseError};
use super::{bounded, protocol, CheckedSession};
use crate::session::reply_budget::{limit, OutputBudget};
use crate::session::{DeviceQuery, QueryStrings};
use std::io;

impl CheckedSession<'_> {
    pub(crate) async fn get_value(
        &mut self,
        query: DeviceQuery,
        output: &mut OutputBudget,
    ) -> Result<QueryStrings, LockdownError> {
        let mut stream = self
            .stream
            .take()
            .ok_or_else(|| protocol("checked existing-trust session has been retired"))?;
        let result = self
            .context
            .run(async {
                send_lockdown(
                    &mut stream,
                    &GetValueRequest {
                        label: "ios-rs",
                        request: "GetValue",
                        domain: None,
                        key: query.key(),
                    },
                )
                .await?;
                let mut reply: plist::Dictionary =
                    bounded::recv(&mut stream, self.budget.as_ref()).await?;
                // Error is authoritative even if success fields are also present.
                if let Some(error) = reply.remove("Error") {
                    let plist::Value::String(code) = error else {
                        return Err(protocol("checked GetValue Error was not a string"));
                    };
                    return Err(LockdownError::Io(io::Error::other(
                        LockdownResponseError::new("GetValue", code),
                    )));
                }
                if let Some(request) = reply.get("Request") {
                    if request.as_string() != Some("GetValue") {
                        return Err(protocol("checked GetValue received an unrelated response"));
                    }
                }
                extract(reply.remove("Value"), query, output)
            })
            .await;
        match result {
            Ok(value) => {
                self.stream = Some(stream);
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }
}

fn extract(
    value: Option<plist::Value>,
    query: DeviceQuery,
    output: &mut OutputBudget,
) -> Result<QueryStrings, LockdownError> {
    let present = value.is_some();
    let fields = query.fields();
    let (product_type, product_version, device_name) = match value.as_ref() {
        None => (None, None, None),
        Some(value) if query.key().is_some() => {
            let value = value
                .as_string()
                .ok_or_else(|| protocol("checked GetValue selected field was not a string"))?;
            (
                fields.includes_product_type().then_some(value),
                fields.includes_product_version().then_some(value),
                fields.includes_device_name().then_some(value),
            )
        }
        Some(value) => {
            let dictionary = value
                .as_dictionary()
                .ok_or_else(|| protocol("checked root GetValue Value was not a dictionary"))?;
            let field = |selected: bool, key: &str| {
                if !selected {
                    return Ok(None);
                }
                dictionary
                    .get(key)
                    .map(|value| {
                        value.as_string().ok_or_else(|| {
                            protocol("checked root GetValue selected field was not a string")
                        })
                    })
                    .transpose()
            };
            (
                field(fields.includes_product_type(), "ProductType")?,
                field(fields.includes_product_version(), "ProductVersion")?,
                field(fields.includes_device_name(), "DeviceName")?,
            )
        }
    };
    let bytes = [product_type, product_version, device_name]
        .into_iter()
        .flatten()
        .try_fold(0usize, |bytes, value| {
            bytes
                .checked_add(value.len())
                .ok_or_else(|| limit("device query retained UTF-8 length overflow"))
        })?;
    output.admit(bytes)?;
    Ok(QueryStrings {
        value_present: present,
        product_type: product_type.map(str::to_owned),
        product_version: product_version.map(str::to_owned),
        device_name: device_name.map(str::to_owned),
    })
}

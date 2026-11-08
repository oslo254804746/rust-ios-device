//! Additive reply limits; unbudgeted checked sessions retain their old path.

use super::super::{recv_lockdown, LockdownError};
use crate::session::reply_budget::ReplyBudget;
use std::{io, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt};

pub(super) async fn recv<R, T>(
    reader: &mut R,
    budget: Option<&Arc<ReplyBudget>>,
) -> Result<T, LockdownError>
where
    R: AsyncRead + Unpin,
    T: for<'de> serde::Deserialize<'de>,
{
    let Some(budget) = budget else {
        return recv_lockdown(reader).await;
    };
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await?;
    let body = usize::try_from(u32::from_be_bytes(prefix)).map_err(|error| {
        super::super::error_context::native(
            io::ErrorKind::InvalidData,
            "decode SDK lockdown frame length",
            error,
        )
    })?;
    budget.lockdown(body)?;
    let mut payload = vec![0; body];
    reader.read_exact(&mut payload).await?;
    plist::from_bytes(&payload).map_err(|error| {
        super::super::error_context::native(
            io::ErrorKind::InvalidData,
            "decode SDK lockdown response plist",
            error,
        )
    })
}

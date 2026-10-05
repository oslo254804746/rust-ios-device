//! Owned session state and independent cleanup shared by finite and interactive tasks.
use super::control::{self, Budget};
use anyhow::{Context, Result};
use ios_core::{
    device::ServiceStream,
    webinspector::{AutomationSession, InspectorSession, WebInspectorClient},
};
use std::time::Duration;

pub(super) const CLEANUP_BUDGET: Duration = Duration::from_secs(5);

pub(in crate::cmd::webinspector) enum Session {
    None,
    Inspector {
        session: InspectorSession,
        attached: bool,
    },
    Automation {
        session: AutomationSession,
        attached: bool,
        application_id: String,
    },
}

pub(super) fn cap(
    client: &mut WebInspectorClient<ServiceStream>,
    budget: Budget,
) -> Result<Duration> {
    let remaining = budget.remaining()?;
    client.set_operation_timeout(Some(remaining));
    Ok(remaining)
}

pub(in crate::cmd::webinspector) async fn cleanup(
    client: &mut WebInspectorClient<ServiceStream>,
    owner: &mut Session,
    duration: Duration,
) -> Result<()> {
    if !client.is_usable()
        || matches!(
            owner,
            Session::None
                | Session::Inspector {
                    attached: false,
                    ..
                }
                | Session::Automation {
                    attached: false,
                    ..
                }
        )
    {
        return Ok(());
    }
    let budget = Budget::new(duration, "cleanup")?;
    let result = match owner {
        Session::Inspector { session, .. } => {
            budget
                .drive(
                    Box::pin(async {
                        cap(client, budget)?;
                        client
                            .send_socket_teardown(
                                session.session_id(),
                                session.application_id(),
                                session.page_id(),
                            )
                            .await?;
                        Ok(())
                    }),
                    std::future::pending::<std::io::Result<()>>(),
                )
                .await
        }
        Session::Automation {
            session,
            application_id,
            ..
        } => {
            let stop = budget
                .drive(
                    Box::pin(async {
                        cap(client, budget)?;
                        session
                            .stop_session(client)
                            .await
                            .map_err(anyhow::Error::from)
                    }),
                    std::future::pending::<std::io::Result<()>>(),
                )
                .await;
            let socket = if client.is_usable() {
                budget
                    .drive(
                        Box::pin(async {
                            cap(client, budget)?;
                            client
                                .send_socket_teardown(
                                    session.session_id(),
                                    application_id,
                                    session.page_id(),
                                )
                                .await
                                .map_err(anyhow::Error::from)
                        }),
                        std::future::pending::<std::io::Result<()>>(),
                    )
                    .await
            } else {
                Ok(())
            };
            control::finish(stop, socket)
        }
        Session::None => Ok(()),
    };
    result.context("webinspector session cleanup failed")
}

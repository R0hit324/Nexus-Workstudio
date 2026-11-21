use std::sync::Arc;

use nexus_core::cube::{CreateCubeRequest, CubeDescriptor, CubeManager, JoinCubeRequest};
use nexus_core::event::{Event, EventPayload};
use nexus_core::ids::{ConnectionId, CubeId, RequestId, SessionId, UserId};
use nexus_core::timestamp::Timestamp;
use nexus_core::workspace::Workspace;
use nexus_protocol::message::{ErrorCode as ProtoError, Message};

use crate::build::BuildService;
use crate::connection::ConnectionManager;
use crate::events::EventRouter;
use crate::metrics::Metrics;
use crate::permissions::{CubeAction, PermissionManager};
use crate::plugins::PluginHost;
use crate::session::SessionManager;
use crate::terminals::TerminalService;
use crate::workspace::WorkspaceManager;

/// Per-request context derived from the live session.
#[derive(Debug, Clone, Copy)]
pub struct DispatchContext {
    pub session_id: SessionId,
    pub user_id: UserId,
    pub connection_id: ConnectionId,
}

/// What the connection task should do after handling one request.
#[derive(Debug)]
pub enum DispatchAction {
    /// Send this reply to the peer.
    Reply(Message),
    /// Proceed without sending anything (e.g. broadcast-only events).
    Ack,
    /// Tear the connection down (client asked to disconnect).
    Close,
}

/// Central request dispatcher. Every dispatcher run is thin: it validates
/// session + cube isolation, delegates to the domain managers, and shapes the
/// reply message. No networking happens here.
pub struct Dispatcher {
    cube_manager: Arc<dyn CubeManager>,
    session_manager: Arc<SessionManager>,
    connection_manager: Arc<ConnectionManager>,
    permission_manager: Arc<PermissionManager>,
    workspace_manager: Arc<WorkspaceManager>,
    event_router: Arc<EventRouter>,
    metrics: Arc<Metrics>,
    build_service: Arc<BuildService>,
    terminal_service: Arc<TerminalService>,
    plugins: std::sync::RwLock<Option<Arc<PluginHost>>>,
}

impl Dispatcher {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cube_manager: Arc<dyn CubeManager>,
        session_manager: Arc<SessionManager>,
        connection_manager: Arc<ConnectionManager>,
        permission_manager: Arc<PermissionManager>,
        workspace_manager: Arc<WorkspaceManager>,
        event_router: Arc<EventRouter>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            cube_manager,
            session_manager,
            connection_manager: connection_manager.clone(),
            permission_manager,
            workspace_manager,
            event_router,
            metrics,
            build_service: Arc::new(BuildService::new(
                nexus_build::BuildManagerConfig::default(),
                connection_manager.clone(),
            )),
            terminal_service: Arc::new(TerminalService::new(connection_manager)),
            plugins: std::sync::RwLock::new(None),
        }
    }

    /// Attaches the plugin host after boot (the dispatcher is built before
    /// the host starts serving).
    #[must_use]
    pub fn with_plugins(self, host: Arc<PluginHost>) -> Self {
        *self.plugins.write().expect("plugin host set once") = Some(host);
        self
    }

    /// The attached plugin host, if any.
    #[must_use]
    pub fn plugin_host(&self) -> Option<Arc<PluginHost>> {
        self.plugins.read().expect("plugin host lock").clone()
    }

    /// Routes a single client message to the domain and produces the reply.
    pub async fn handle(&self, message: Message, ctx: DispatchContext) -> DispatchAction {
        self.metrics.request_started();
        match message {
            Message::Heartbeat { .. } => DispatchAction::Reply(Message::HeartbeatAck {
                at: Timestamp::now(),
            }),
            Message::CreateCube { request_id, name } => {
                let cube = self
                    .cube_manager
                    .create_cube(CreateCubeRequest {
                        name,
                        owner: ctx.user_id,
                    })
                    .await;
                let reply = result(request_id, cube, |cube| {
                    self.workspace_manager.attach(&cube);
                    self.permission_manager.refresh(&cube);
                    Message::CubeCreated { request_id, cube }
                });
                DispatchAction::Reply(reply)
            }
            Message::ListCubes { request_id } => {
                let list = self.cube_manager.list_cubes().await;
                let reply = result(request_id, list, |all| {
                    let visible: Vec<CubeDescriptor> = all
                        .into_iter()
                        .filter(|cube| cube.contains_member(ctx.user_id))
                        .collect();
                    Message::CubeList {
                        request_id,
                        cubes: visible,
                    }
                });
                DispatchAction::Reply(reply)
            }
            Message::JoinCube {
                request_id,
                cube_id,
            } => {
                let joined = self
                    .cube_manager
                    .join_cube(JoinCubeRequest {
                        cube_id,
                        user_id: ctx.user_id,
                    })
                    .await;
                let reply = result(request_id, joined, |cube| {
                    self.session_manager.join_cube(ctx.session_id, cube_id);
                    self.connection_manager
                        .bind_cube(ctx.connection_id, Some(cube_id));
                    self.permission_manager.refresh(&cube);
                    self.presence(cube_id, ctx.user_id, false, Some(ctx.session_id));
                    Message::CubeJoined { request_id, cube }
                });
                DispatchAction::Reply(reply)
            }
            Message::LeaveCube {
                request_id,
                cube_id,
            } => {
                let left = self
                    .cube_manager
                    .leave_cube(JoinCubeRequest {
                        cube_id,
                        user_id: ctx.user_id,
                    })
                    .await;
                let refreshed = self.cube_for(ctx, cube_id).await.ok().flatten();
                let reply = result(request_id, left, |()| {
                    self.session_manager.leave_cube(ctx.session_id);
                    self.connection_manager.bind_cube(ctx.connection_id, None);
                    if let Some(cube) = refreshed {
                        self.permission_manager.refresh(&cube);
                    }
                    self.presence(cube_id, ctx.user_id, true, Some(ctx.session_id));
                    Message::CubeLeft {
                        request_id,
                        cube_id,
                    }
                });
                DispatchAction::Reply(reply)
            }
            Message::ListWorkspace {
                request_id,
                cube_id,
            } => {
                let reply = self.workspace_list(request_id, ctx, cube_id).await;
                DispatchAction::Reply(reply)
            }
            Message::CreateFile {
                request_id,
                cube_id,
                path,
            } => {
                let reply = self
                    .mutate_file(
                        request_id,
                        ctx,
                        cube_id,
                        &path,
                        None,
                        |workspace, path, event_kind| {
                            let workspace = workspace.clone();
                            let path = path.to_owned();
                            async move {
                                workspace.create_file(&path).await?;
                                Ok((workspace, path, event_kind))
                            }
                        },
                    )
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::ReadFile {
                request_id,
                cube_id,
                path,
            } => {
                let cube = match self.ensure_joined(ctx, cube_id).await {
                    Ok(cube) => cube,
                    Err(err) => return DispatchAction::Reply(error(request_id, err)),
                };
                if let Err(err) =
                    self.permission_manager
                        .authorize(&cube, ctx.user_id, CubeAction::ReadFile)
                {
                    return DispatchAction::Reply(error(request_id, err));
                }
                let workspace = self.workspace_manager.get(cube_id);
                let reply = match workspace {
                    None => error(
                        request_id,
                        nexus_core::error::Error::not_found("workspace not available"),
                    ),
                    Some(workspace) => match workspace.read_file(&path).await {
                        Ok(content) => Message::FileContent {
                            request_id,
                            cube_id,
                            path,
                            version: workspace.version(),
                            content,
                        },
                        Err(err) => error(request_id, err),
                    },
                };
                DispatchAction::Reply(reply)
            }
            Message::WriteFile {
                request_id,
                cube_id,
                path,
                content,
            } => {
                let reply = self
                    .mutate_file(
                        request_id,
                        ctx,
                        cube_id,
                        &path,
                        None,
                        |workspace, path, event_kind| {
                            let workspace = workspace.clone();
                            let path = path.to_owned();
                            let content = content.clone();
                            async move {
                                workspace.write_file(&path, &content).await?;
                                Ok((workspace, path, event_kind))
                            }
                        },
                    )
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::DeleteFile {
                request_id,
                cube_id,
                path,
            } => {
                let reply = self
                    .mutate_file(
                        request_id,
                        ctx,
                        cube_id,
                        &path,
                        None,
                        |workspace, path, event_kind| {
                            let workspace = workspace.clone();
                            let path = path.to_owned();
                            async move {
                                workspace.delete_file(&path).await?;
                                Ok((workspace, path, event_kind))
                            }
                        },
                    )
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::RenameFile {
                request_id,
                cube_id,
                from,
                to,
            } => {
                let reply = self
                    .mutate_file(
                        request_id,
                        ctx,
                        cube_id,
                        &from,
                        Some(&to),
                        |workspace, path, event_kind| {
                            let workspace = workspace.clone();
                            let path = path.to_owned();
                            let to = to.clone();
                            async move {
                                workspace.rename_file(&path, &to).await?;
                                Ok((workspace, path, event_kind))
                            }
                        },
                    )
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::BuildSubmit {
                request_id,
                cube_id,
                target,
                configuration,
            } => {
                let reply = self
                    .build_submit(request_id, ctx, cube_id, target, configuration)
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::BuildCancel {
                request_id,
                build_id,
            } => {
                let reply = match self.build_service.cancel(build_id).await {
                    Ok(()) => Message::BuildCancelled {
                        request_id,
                        build_id,
                    },
                    Err(err) => error(
                        request_id,
                        nexus_core::error::Error::new(
                            nexus_core::error::ErrorCode::Build,
                            format!("build cancel failed: {err}"),
                        ),
                    ),
                };
                DispatchAction::Reply(reply)
            }
            Message::TerminalCreate {
                request_id,
                cube_id,
                shell,
                working_directory,
                size,
                visibility,
            } => {
                let reply = self
                    .terminal_create(
                        request_id,
                        ctx,
                        cube_id,
                        shell,
                        working_directory,
                        size,
                        visibility,
                    )
                    .await;
                DispatchAction::Reply(reply)
            }
            Message::TerminalWrite { terminal_id, data } => {
                // Input traffic is one-way; authorization failures are logged
                // without an ack to preserve the low-latency byte pipe.
                if let Err(err) = self
                    .authorize_terminal(ctx, terminal_id, CubeAction::SendTerminalInput)
                    .await
                {
                    tracing::debug!(%terminal_id, error = %err, "terminal write unauthorized");
                } else if let Err(err) = self.terminal_service.write(terminal_id, &data).await {
                    tracing::debug!(%terminal_id, error = %err, "terminal write failed");
                }
                DispatchAction::Ack
            }
            Message::TerminalResize {
                request_id,
                terminal_id,
                size,
            } => {
                let reply = match self
                    .authorize_terminal(ctx, terminal_id, CubeAction::SendTerminalInput)
                    .await
                {
                    Ok(_) => match self.terminal_service.resize(terminal_id, size).await {
                        Ok(()) => Message::Ack {
                            request_id,
                            ok: true,
                            message: None,
                        },
                        Err(err) => error(
                            request_id,
                            nexus_core::error::Error::new(
                                nexus_core::error::ErrorCode::Terminal,
                                format!("terminal resize failed: {err}"),
                            ),
                        ),
                    },
                    Err(err) => error(request_id, err),
                };
                DispatchAction::Reply(reply)
            }
            Message::TerminalSignal {
                request_id,
                terminal_id,
                signal,
            } => {
                let reply = match self
                    .authorize_terminal(ctx, terminal_id, CubeAction::SendTerminalInput)
                    .await
                {
                    Ok(_) => match self.terminal_service.signal(terminal_id, signal).await {
                        Ok(()) => Message::Ack {
                            request_id,
                            ok: true,
                            message: None,
                        },
                        Err(err) => error(
                            request_id,
                            nexus_core::error::Error::new(
                                nexus_core::error::ErrorCode::Terminal,
                                format!("terminal signal failed: {err}"),
                            ),
                        ),
                    },
                    Err(err) => error(request_id, err),
                };
                DispatchAction::Reply(reply)
            }
            Message::TerminalClose {
                request_id,
                terminal_id,
            } => {
                let reply = match self
                    .authorize_terminal(ctx, terminal_id, CubeAction::SendTerminalInput)
                    .await
                {
                    Ok(_) => match self.terminal_service.close(terminal_id).await {
                        Ok(exit_code) => Message::TerminalClosed {
                            request_id,
                            terminal_id,
                            exit_code,
                        },
                        Err(err) => error(
                            request_id,
                            nexus_core::error::Error::new(
                                nexus_core::error::ErrorCode::Terminal,
                                format!("terminal close failed: {err}"),
                            ),
                        ),
                    },
                    Err(err) => error(request_id, err),
                };
                DispatchAction::Reply(reply)
            }
            Message::ListTerminals {
                request_id,
                cube_id,
            } => {
                let cube = match self.ensure_joined(ctx, cube_id).await {
                    Ok(cube) => cube,
                    Err(err) => return DispatchAction::Reply(error(request_id, err)),
                };
                if let Err(err) =
                    self.permission_manager
                        .authorize(&cube, ctx.user_id, CubeAction::ViewCube)
                {
                    return DispatchAction::Reply(error(request_id, err));
                }
                let terminals = self.terminal_service.list(cube_id).await;
                DispatchAction::Reply(Message::TerminalList {
                    request_id,
                    cube_id,
                    terminals,
                })
            }
            Message::PluginStatusRequest { request_id } => {
                let Some(plugins) = self.plugin_host() else {
                    return DispatchAction::Reply(Message::error(
                        Some(request_id),
                        ProtoError::Unsupported,
                        "plugin host is not running",
                    ));
                };
                let records: Vec<nexus_protocol::PluginRecordInfo> = plugins
                    .manager()
                    .registry()
                    .all()
                    .iter()
                    .map(|record| nexus_protocol::PluginRecordInfo {
                        id: record.manifest.id.as_str().to_owned(),
                        name: record.manifest.name.clone(),
                        version: record.manifest.version.to_string(),
                        state: format!("{:?}", record.state).to_lowercase(),
                        trust: format!("{:?}", record.trust).to_lowercase(),
                        error: record.last_error.clone(),
                    })
                    .collect();
                DispatchAction::Reply(Message::PluginStatus {
                    request_id,
                    plugins: records,
                })
            }
            Message::Disconnect { .. } => DispatchAction::Close,
            Message::Event { .. } | Message::EventBatch { .. } => {
                DispatchAction::Reply(Message::error(
                    None,
                    ProtoError::InvalidRequest,
                    "client may not send events; events flow server-to-client",
                ))
            }
            other => {
                tracing::warn!(message = ?std::mem::discriminant(&other), "unexpected_message_from_client");
                DispatchAction::Reply(Message::error(
                    None,
                    ProtoError::InvalidRequest,
                    "message not valid from client",
                ))
            }
        }
    }

    // ---- Request implementations --------------------------------------------

    /// Submits a build for a Cube. Reply correlates the assigned build id.
    async fn build_submit(
        &self,
        request_id: RequestId,
        ctx: DispatchContext,
        cube_id: CubeId,
        target: nexus_core::BuildTarget,
        configuration: nexus_core::BuildConfiguration,
    ) -> Message {
        let cube = match self.ensure_joined(ctx, cube_id).await {
            Ok(cube) => cube,
            Err(err) => return error(request_id, err),
        };
        if let Err(err) =
            self.permission_manager
                .authorize(&cube, ctx.user_id, CubeAction::RunBuild)
        {
            return error(request_id, err);
        }
        match self
            .build_service
            .submit(cube_id, ctx.user_id, target, configuration)
            .await
        {
            Ok(build_id) => Message::BuildQueued {
                request_id,
                build_id,
                queue_position: self.build_service.queued_count().await as u32,
            },
            Err(err) => error(
                request_id,
                nexus_core::error::Error::new(
                    nexus_core::error::ErrorCode::Build,
                    format!("build submission failed: {err}"),
                ),
            ),
        }
    }

    /// Creates and starts a terminal session for the requesting session.
    #[allow(clippy::too_many_arguments)]
    async fn terminal_create(
        &self,
        request_id: RequestId,
        ctx: DispatchContext,
        cube_id: CubeId,
        shell: Option<String>,
        working_directory: String,
        size: nexus_core::TerminalSize,
        visibility: nexus_core::TerminalVisibility,
    ) -> Message {
        let cube = match self.ensure_joined(ctx, cube_id).await {
            Ok(cube) => cube,
            Err(err) => return error(request_id, err),
        };
        if let Err(err) =
            self.permission_manager
                .authorize(&cube, ctx.user_id, CubeAction::CreateTerminal)
        {
            return error(request_id, err);
        }
        match self
            .terminal_service
            .create(
                cube_id,
                ctx.user_id,
                ctx.session_id,
                shell,
                working_directory,
                size,
                visibility,
            )
            .await
        {
            Ok(session) => Message::TerminalCreated {
                request_id,
                session,
            },
            Err(err) => error(
                request_id,
                nexus_core::error::Error::new(
                    nexus_core::error::ErrorCode::Terminal,
                    format!("terminal creation failed: {err}"),
                ),
            ),
        }
    }

    /// Resolves a terminal and enforces the join-isolation rule against the
    /// Cube that owns it.
    async fn authorize_terminal(
        &self,
        ctx: DispatchContext,
        terminal_id: nexus_core::ids::TerminalId,
        action: CubeAction,
    ) -> nexus_core::Result<nexus_core::TerminalSession> {
        let session = self
            .terminal_service
            .get(terminal_id)
            .await
            .ok_or_else(|| {
                nexus_core::error::Error::not_found(format!("terminal {terminal_id} not found"))
            })?;
        let cube = self.ensure_joined(ctx, session.cube_id).await?;
        self.permission_manager
            .authorize(&cube, ctx.user_id, action)?;
        Ok(session)
    }

    async fn workspace_list(
        &self,
        request_id: RequestId,
        ctx: DispatchContext,
        cube_id: CubeId,
    ) -> Message {
        let cube = match self.ensure_joined(ctx, cube_id).await {
            Ok(cube) => cube,
            Err(err) => return error(request_id, err),
        };
        if let Err(err) =
            self.permission_manager
                .authorize(&cube, ctx.user_id, CubeAction::ListWorkspace)
        {
            return error(request_id, err);
        }
        let Some(workspace) = self.workspace_manager.get(cube_id) else {
            return error(
                request_id,
                nexus_core::error::Error::not_found("workspace not available"),
            );
        };
        match (workspace.metadata().await, workspace.list_files().await) {
            (Ok(workspace_meta), Ok(files)) => Message::WorkspaceList {
                request_id,
                cube_id,
                workspace: workspace_meta,
                files,
            },
            (Err(err), _) | (_, Err(err)) => error(request_id, err),
        }
    }

    /// Executes a workspace mutation under the join-isolation + permission
    /// guards, then publishes the derived content event to cube members.
    #[allow(clippy::type_complexity)]
    async fn mutate_file<F, Fut>(
        &self,
        request_id: RequestId,
        ctx: DispatchContext,
        cube_id: CubeId,
        path: &str,
        rename_to: Option<&str>,
        mutate: F,
    ) -> Message
    where
        F: FnOnce(Arc<crate::workspace::CubeWorkspace>, &str, EventPayload) -> Fut,
        Fut: std::future::Future<
            Output = nexus_core::Result<(
                Arc<crate::workspace::CubeWorkspace>,
                String,
                EventPayload,
            )>,
        >,
    {
        let cube = match self.ensure_joined(ctx, cube_id).await {
            Ok(cube) => cube,
            Err(err) => return error(request_id, err),
        };
        let action = match path {
            _ if rename_to.is_some() => CubeAction::RenameFile,
            _ => CubeAction::WriteFile,
        };
        if let Err(err) = self
            .permission_manager
            .authorize(&cube, ctx.user_id, action)
        {
            return error(request_id, err);
        }
        let Some(workspace) = self.workspace_manager.get(cube_id) else {
            return error(
                request_id,
                nexus_core::error::Error::not_found("workspace not available"),
            );
        };
        let event_kind = match action {
            CubeAction::RenameFile => EventPayload::FileRenamed {
                from: path.to_owned(),
                to: rename_to.unwrap_or(path).to_owned(),
            },
            _ => EventPayload::FileEdited {
                path: path.to_owned(),
                version: workspace.version(),
            },
        };

        match mutate(workspace.clone(), path, event_kind).await {
            Ok((workspace, path, payload)) => {
                self.node_content_event(cube_id, ctx, &path, payload);
                self.metrics.workspace_operation();
                let _ = workspace;
                Message::Ack {
                    request_id,
                    ok: true,
                    message: None,
                }
            }
            Err(err) => error(request_id, err),
        }
    }

    // ---- Shared helpers -----------------------------------------------------

    /// Resolves the cube and enforces the join-isolation rule: workspace
    /// operations are only allowed on the Cube the session is currently in.
    async fn ensure_joined(
        &self,
        ctx: DispatchContext,
        cube_id: CubeId,
    ) -> nexus_core::Result<CubeDescriptor> {
        let session = self
            .session_manager
            .get(ctx.session_id)
            .ok_or_else(|| nexus_core::error::Error::authentication("session not found"))?;
        if session.cube_id() != Some(cube_id) {
            return Err(nexus_core::error::Error::authorization(format!(
                "session {} is not joined to cube {cube_id}",
                ctx.session_id
            )));
        }
        self.cube_for(ctx, cube_id).await?.ok_or_else(|| {
            nexus_core::error::Error::not_found(format!("cube {cube_id} does not exist"))
        })
    }

    async fn cube_for(
        &self,
        _ctx: DispatchContext,
        cube_id: CubeId,
    ) -> nexus_core::Result<Option<CubeDescriptor>> {
        self.cube_manager.get_cube(cube_id).await
    }

    /// Sends a presence event to a Cube's members (join/leave). Public so the
    /// connection task can emit a leave event on disconnect. `exclude_session`
    /// suppresses delivery to that session (used to avoid echoing a join/leave
    /// back to the actor).
    pub fn presence(
        &self,
        cube_id: CubeId,
        user_id: UserId,
        leaving: bool,
        exclude_session: Option<SessionId>,
    ) {
        let payload = if leaving {
            EventPayload::UserLeft { user_id }
        } else {
            EventPayload::UserJoined { user_id }
        };
        let event = Event::new(cube_id, user_id, payload);
        if let Err(err) = self.event_router.dispatch(cube_id, event, exclude_session) {
            tracing::error!(%err, cube_id = %cube_id, "presence event dispatch failed");
        }
    }

    pub fn node_content_event(
        &self,
        cube_id: CubeId,
        ctx: DispatchContext,
        _path: &str,
        payload: EventPayload,
    ) {
        let event = Event::new(cube_id, ctx.user_id, payload);
        if let Err(err) = self
            .event_router
            .dispatch(cube_id, event, Some(ctx.session_id))
        {
            tracing::error!(%err, cube_id = %cube_id, "content event dispatch failed");
        }
    }
}

/// Maps a domain success into the success message, or a correlated error.
fn result<T>(
    request_id: RequestId,
    outcome: nexus_core::Result<T>,
    ok: impl FnOnce(T) -> Message,
) -> Message {
    match outcome {
        Ok(value) => ok(value),
        Err(err) => error(request_id, err),
    }
}

/// Builds a correlated protocol error from a domain error.
#[must_use]
pub fn error(request_id: RequestId, err: nexus_core::error::Error) -> Message {
    Message::error(
        Some(request_id),
        to_protocol_error(err.code()),
        err.user_message(),
    )
}

/// Translates core error codes onto the protocol's stable error surface.
#[must_use]
pub fn to_protocol_error(code: nexus_core::error::ErrorCode) -> ProtoError {
    use nexus_core::error::ErrorCode as Core;
    match code {
        Core::Authentication => ProtoError::AuthenticationFailed,
        Core::Authorization => ProtoError::PermissionDenied,
        Core::Protocol => ProtoError::ProtocolViolation,
        Core::NotFound => ProtoError::NotFound,
        Core::InvalidRequest => ProtoError::InvalidRequest,
        Core::AlreadyExists => ProtoError::AlreadyExists,
        Core::RateLimited => ProtoError::RateLimited,
        Core::InvalidPath => ProtoError::InvalidPath,
        Core::Network => ProtoError::Timeout,
        Core::Workspace | Core::Storage => ProtoError::InvalidRequest,
        _ => ProtoError::InternalError,
    }
}

impl DispatchAction {
    #[must_use]
    pub const fn reply(&self) -> Option<&Message> {
        match self {
            Self::Reply(message) => Some(message),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_close(&self) -> bool {
        matches!(self, Self::Close)
    }
}

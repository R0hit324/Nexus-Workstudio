//! Diff-first AI editing (`§27`–`§30`, `§40`).
//!
//! The AI never mutates workspace state directly (`§85.3`): it *proposes*
//! edits, which are validated, previewed, approved, and then applied through
//! the existing synchronization engine via the [`Applier`] boundary (`§29`).
//! Multi-file operations are transactions: preview, apply, rollback (`§40`).

use crate::permission::AiPermission;
use crate::tools::ToolDefinition;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Unified diff generation (`§28`)
// ---------------------------------------------------------------------------

/// Renders a line-based unified-ish diff between two file contents.
/// Hunks are simple "+/-" pairs; context lines keep bullets small.
#[must_use]
pub fn unified_diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();

    // LCS table (typical workspace files make O(n*m) fine).
    let n = a.len();
    let m = b.len();
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(' ');
            out.push_str(a[i]);
            out.push('\n');
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push('-');
            out.push_str(a[i]);
            out.push('\n');
            i += 1;
        } else {
            out.push('+');
            out.push_str(b[j]);
            out.push('\n');
            j += 1;
        }
    }
    for line in &a[i..] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &b[j..] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_owned()
}

// ---------------------------------------------------------------------------
// Transactions (`§40`)
// ---------------------------------------------------------------------------

/// One file-level modification inside a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedEdit {
    /// Workspace-relative path.
    pub path: String,
    /// The content the proposal was computed against (conflict detection).
    pub base: Option<String>,
    /// Target content.
    pub new_text: String,
    /// Operation class.
    pub operation: EditOperation,
}

/// File operations supported by the staging layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    /// Replace the file body (diff-first path).
    Write,
    /// Create a new file.
    Create,
    /// Delete a file. Stronger approval (`§30`).
    Delete,
    /// Move a file.
    Rename(String),
}

impl EditOperation {
    /// The AI permission this modification exercises.
    #[must_use]
    pub const fn permission(&self) -> AiPermission {
        match self {
            Self::Write | Self::Rename(_) => AiPermission::ProposeEdit,
            Self::Create => AiPermission::CreateFiles,
            Self::Delete => AiPermission::DeleteFiles,
        }
    }

    /// Deletions and commands are the destructive tiers (`§30`).
    #[must_use]
    pub const fn requires_approval(&self) -> bool {
        matches!(self, Self::Delete)
    }
}

/// Transaction lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Proposed,
    Approved,
    Rejected,
    Applied,
    RolledBack,
    Failed,
}

/// A grouped, staged set of edits (one logical AI fix, `§40`).
#[derive(Debug, Clone)]
pub struct EditTransaction {
    pub id: u64,
    pub name: String,
    pub edits: Vec<ProposedEdit>,
    pub state: TransactionState,
    /// Snapshot of pre-apply content, captured at apply-time.
    undo: BTreeMap<String, Option<String>>,
}

impl EditTransaction {
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>, edits: Vec<ProposedEdit>) -> Self {
        Self {
            id,
            name: name.into(),
            edits,
            state: TransactionState::Proposed,
            undo: BTreeMap::new(),
        }
    }

    /// Whether every edit carries the approval requirement (any Delete).
    #[must_use]
    pub fn needs_approval(&self) -> bool {
        self.edits
            .iter()
            .any(|edit| edit.operation.requires_approval())
    }
}

/// Workspace mutation boundary — the host wires this to the Part 5
/// synchronization engine so approved AI edits behave like normal user
/// edits (`§29`).
pub trait Applier: Send + Sync {
    /// Current content of one file; `Err` when it does not exist.
    fn read(&self, path: &str) -> Result<String, String>;
    /// Full-body write of one file.
    fn write(&self, path: &str, content: &str) -> Result<(), String>;
    /// Remove one file.
    fn delete(&self, path: &str) -> Result<(), String>;
    /// Move one file.
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
}

/// Failure of a staging action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagingFailure {
    UnknownTransaction(u64),
    NotInState(TransactionState, TransactionState),
    StaleBase(String),
    WorkspaceError(String),
    NothingToApply,
}

impl core::fmt::Display for StagingFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownTransaction(id) => write!(f, "unknown transaction {id}"),
            Self::NotInState(expected, actual) => {
                write!(f, "transaction is {actual:?}, needs {expected:?}")
            }
            Self::StaleBase(path) => {
                write!(f, "workspace drifted for {path} since the proposal")
            }
            Self::WorkspaceError(message) => write!(f, "workspace rejected the edit: {message}"),
            Self::NothingToApply => write!(f, "transaction carries no edits"),
        }
    }
}

/// Result of an applied transaction.
#[derive(Debug, Clone)]
pub struct AppliedTransaction {
    pub id: u64,
    /// Paths written (created or replaced).
    pub written: Vec<String>,
    /// Paths deleted.
    pub deleted: Vec<String>,
    /// (from, to) renames.
    pub renamed: Vec<(String, String)>,
}

impl AppliedTransaction {
    /// Number of individual file effects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.written.len() + self.deleted.len() + self.renamed.len()
    }

    /// Whether the transaction had no effects at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The server-side ledger for staged AI actions.
#[derive(Debug, Default)]
pub struct EditStaging {
    transactions: BTreeMap<u64, EditTransaction>,
    next_id: u64,
}

impl EditStaging {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stages a new proposal; ids are monotonic.
    pub fn propose(&mut self, name: impl Into<String>, edits: Vec<ProposedEdit>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.transactions
            .insert(id, EditTransaction::new(id, name, edits));
        id
    }

    /// Reads one transaction for preview (`§28` Approve/Reject/Edit decision).
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&EditTransaction> {
        self.transactions.get(&id)
    }

    /// Approves a proposal; legal from `Proposed` state.
    pub fn approve(&mut self, id: u64) -> Result<(), StagingFailure> {
        self.transition(id, TransactionState::Proposed, TransactionState::Approved)
    }

    /// Rejects a proposal; legal while it has not been applied yet.
    pub fn reject(&mut self, id: u64) -> Result<(), StagingFailure> {
        self.transition(id, TransactionState::Proposed, TransactionState::Rejected)
    }

    fn transition(
        &mut self,
        id: u64,
        from: TransactionState,
        to: TransactionState,
    ) -> Result<(), StagingFailure> {
        let transaction = self
            .transactions
            .get_mut(&id)
            .ok_or(StagingFailure::UnknownTransaction(id))?;
        if transaction.state != from {
            return Err(StagingFailure::NotInState(transaction.state, from));
        }
        transaction.state = to;
        Ok(())
    }

    /// Applies an approved transaction: **all** bases are validated before
    /// any mutation, then effects run in order; a failure mid-apply rolls
    /// back the pieces already applied (`§40` — no partial workspaces).
    /// Permissions and approvals are enforced server-side beforehand.
    ///
    /// # Errors
    /// `StaleBase`, `NotInState`, or an `Applier` error (with rollback).
    pub fn apply(
        &mut self,
        transaction_id: u64,
        applier: &dyn Applier,
    ) -> Result<AppliedTransaction, StagingFailure> {
        let edits = {
            let transaction = self
                .transactions
                .get(&transaction_id)
                .ok_or(StagingFailure::UnknownTransaction(transaction_id))?;
            if transaction.state != TransactionState::Approved {
                return Err(StagingFailure::NotInState(
                    transaction.state,
                    TransactionState::Approved,
                ));
            }
            if transaction.edits.is_empty() {
                return Err(StagingFailure::NothingToApply);
            }
            transaction.edits.clone()
        };

        // Conflict validation first so nothing moves when bases drifted
        // (§76 "conflict handling" editing test).
        for edit in &edits {
            if let Some(base) = &edit.base {
                let current = applier
                    .read(&edit.path)
                    .map_err(StagingFailure::WorkspaceError)?;
                if current != *base {
                    return Err(StagingFailure::StaleBase(edit.path.clone()));
                }
            }
        }

        // Sequential apply with captured undo state; on any failure, undo
        // the effects that already landed.
        let mut applied = AppliedTransaction {
            id: transaction_id,
            written: Vec::new(),
            deleted: Vec::new(),
            renamed: Vec::new(),
        };
        for edit in &edits {
            let before = applier.read(&edit.path).ok();
            self.transactions
                .get_mut(&transaction_id)
                .expect("state validated above")
                .undo
                .insert(edit.path.clone(), before);

            let result = match &edit.operation {
                EditOperation::Write | EditOperation::Create => applier
                    .write(&edit.path, &edit.new_text)
                    .map(|()| applied.written.push(edit.path.clone()))
                    .map_err(StagingFailure::WorkspaceError),
                EditOperation::Delete => applier
                    .delete(&edit.path)
                    .map(|()| applied.deleted.push(edit.path.clone()))
                    .map_err(StagingFailure::WorkspaceError),
                EditOperation::Rename(to) => applier
                    .rename(&edit.path, to)
                    .map(|()| {
                        applied.renamed.push((edit.path.clone(), to.clone()));
                    })
                    .map_err(StagingFailure::WorkspaceError),
            };
            if let Err(failure) = result {
                let _ = failure;
                return Err(self.partial_rollback(transaction_id, applier, applied));
            }
        }

        self.transition(
            transaction_id,
            TransactionState::Approved,
            TransactionState::Applied,
        )
        .expect("state validated above");
        Ok(applied)
    }

    /// Restores pre-apply contents for every effect of the transaction.
    /// Returns the failure that caused the rollback, for context.
    fn partial_rollback(
        &mut self,
        transaction_id: u64,
        applier: &dyn Applier,
        applied: AppliedTransaction,
    ) -> StagingFailure {
        self.transition(
            transaction_id,
            TransactionState::Approved,
            TransactionState::Failed,
        )
        .expect("state validated in apply");

        // Reverse order: rename undo reverts to, deletes re-materialize.
        for (from, to) in applied.renamed.iter().rev() {
            let _ = applier.rename(to, from);
        }
        for path in applied.deleted.iter().rev() {
            let content = self
                .transactions
                .get(&transaction_id)
                .and_then(|t| t.undo.get(path))
                .and_then(|c| c.clone());
            if let Some(content) = content {
                let _ = applier.write(path, &content);
            }
        }
        for path in applied.written.iter().rev() {
            let content = self
                .transactions
                .get(&transaction_id)
                .and_then(|t| t.undo.get(path))
                .and_then(|c| c.clone());
            let _ = match content {
                Some(content) => applier.write(path, &content),
                None => applier.delete(path),
            };
        }
        StagingFailure::WorkspaceError(String::from("apply failed and was rolled back"))
    }

    /// Rolls a successfully applied transaction back — the human "Undo"
    /// affordance (`§40`).
    ///
    /// # Errors
    /// `NotInState` when the transaction never applied.
    pub fn rollback(
        &mut self,
        transaction_id: u64,
        applier: &dyn Applier,
    ) -> Result<(), StagingFailure> {
        let applied = self
            .transactions
            .get(&transaction_id)
            .filter(|t| t.state == TransactionState::Applied)
            .ok_or(StagingFailure::NotInState(
                TransactionState::Applied,
                TransactionState::Proposed,
            ))?;
        let undo = applied.undo.clone();

        for (path, before) in undo.iter() {
            match before {
                Some(content) => {
                    applier
                        .write(path, content)
                        .map_err(StagingFailure::WorkspaceError)?;
                }
                None => {
                    applier
                        .delete(path)
                        .map_err(StagingFailure::WorkspaceError)?;
                }
            }
        }
        self.transition(
            transaction_id,
            TransactionState::Applied,
            TransactionState::RolledBack,
        )
    }
}

// ---------------------------------------------------------------------------
// Editor tools (`§27`, `§28`)
// ---------------------------------------------------------------------------

/// Handle shared by all editing tools; the host hands the same staging ledger
/// to the human preview loop.
#[must_use]
pub fn shared_staging() -> Arc<std::sync::Mutex<EditStaging>> {
    Arc::new(std::sync::Mutex::new(EditStaging::new()))
}

fn staged_definition(
    name: &'static str,
    description: &str,
    parameters: &str,
    permission: AiPermission,
) -> ToolDefinition {
    ToolDefinition {
        name,
        description: description.into(),
        parameters: parameters.into(),
        permission,
        timeout: std::time::Duration::from_secs(5),
        output_limit: 8_192,
    }
}

/// `propose_edit` (`§28`): diff-first. Validates that `find` occurs in the
/// current snapshot, then stages a `Write` proposal carrying the unified
/// diff and the base it was computed against (for conflict detection).
pub struct ProposeEditTool {
    workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
    staging: Arc<std::sync::Mutex<EditStaging>>,
}

impl ProposeEditTool {
    pub fn new(
        workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
        staging: Arc<std::sync::Mutex<EditStaging>>,
    ) -> Self {
        Self { workspace, staging }
    }
}

#[async_trait]
impl crate::tools::AiTool for ProposeEditTool {
    fn definition(&self) -> ToolDefinition {
        staged_definition(
            "propose_edit",
            "propose a diff-first text replacement in one file (never applied directly)",
            r#"{"path": "string", "find": "string", "replace": "string"}"#,
            AiPermission::ProposeEdit,
        )
    }

    async fn execute(
        &self,
        _context: &crate::tools::ToolContext,
        arguments: &crate::tools::ToolArguments,
    ) -> Result<String, String> {
        let path = arguments.string("path").ok_or("missing `path`")?;
        let find = arguments.string("find").ok_or("missing `find`")?;
        let replace = arguments.string("replace").ok_or("missing `replace`")?;
        let content = self
            .workspace
            .read(path)
            .ok_or_else(|| format!("no such file: {path}"))?;
        let occurrences = content.matches(find).count();
        if occurrences != 1 {
            return Err(format!(
                "`find` must occur exactly once in {path}, found {occurrences} times"
            ));
        }
        let new_text = content.replacen(find, replace, 1);
        let diff = unified_diff(&content, &new_text);
        let id = self.staging.lock().expect("staging lock").propose(
            "propose_edit",
            vec![ProposedEdit {
                path: path.into(),
                base: Some(content),
                new_text,
                operation: EditOperation::Write,
            }],
        );
        Ok(format!("transaction {id} staged\n```diff\n{diff}\n```"))
    }
}

/// `create_file` (`§30`): stages a brand-new file proposal.
pub struct CreateFileTool {
    workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
    staging: Arc<std::sync::Mutex<EditStaging>>,
}

impl CreateFileTool {
    pub fn new(
        workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
        staging: Arc<std::sync::Mutex<EditStaging>>,
    ) -> Self {
        Self { workspace, staging }
    }
}

#[async_trait]
impl crate::tools::AiTool for CreateFileTool {
    fn definition(&self) -> ToolDefinition {
        staged_definition(
            "create_file",
            "propose creating a new workspace file",
            r#"{"path": "string", "content": "string"}"#,
            AiPermission::CreateFiles,
        )
    }

    async fn execute(
        &self,
        _context: &crate::tools::ToolContext,
        arguments: &crate::tools::ToolArguments,
    ) -> Result<String, String> {
        let path = arguments.string("path").ok_or("missing `path`")?;
        let content = arguments.string("content").ok_or("missing `content`")?;
        if self.workspace.read(path).is_some() {
            return Err(format!("{path} already exists"));
        }
        let id = self.staging.lock().expect("staging lock").propose(
            "create_file",
            vec![ProposedEdit {
                path: path.into(),
                base: None,
                new_text: content.into(),
                operation: EditOperation::Create,
            }],
        );
        Ok(format!(
            "transaction {id} staged: new file {path} ({} lines)",
            content.lines().count()
        ))
    }
}

/// `delete_file` (`§30`): destructive, so the registry's approval gate runs
/// before this tool is ever reached; the staged proposal keeps the stronger
/// confirm/undo band.
pub struct DeleteFileTool {
    workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
    staging: Arc<std::sync::Mutex<EditStaging>>,
}

impl DeleteFileTool {
    pub fn new(
        workspace: Arc<dyn crate::tools::WorkspaceSnapshot>,
        staging: Arc<std::sync::Mutex<EditStaging>>,
    ) -> Self {
        Self { workspace, staging }
    }
}

#[async_trait]
impl crate::tools::AiTool for DeleteFileTool {
    fn definition(&self) -> ToolDefinition {
        staged_definition(
            "delete_file",
            "propose deleting one workspace file (requires approval)",
            r#"{"path": "string"}"#,
            AiPermission::DeleteFiles,
        )
    }

    async fn execute(
        &self,
        _context: &crate::tools::ToolContext,
        arguments: &crate::tools::ToolArguments,
    ) -> Result<String, String> {
        let path = arguments.string("path").ok_or("missing `path`")?;
        let content = self
            .workspace
            .read(path)
            .ok_or_else(|| format!("no such file: {path}"))?;
        let id = self.staging.lock().expect("staging lock").propose(
            "delete_file",
            vec![ProposedEdit {
                path: path.into(),
                base: Some(content),
                new_text: String::new(),
                operation: EditOperation::Delete,
            }],
        );
        Ok(format!(
            "transaction {id} staged: delete {path} (approval pending apply)"
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests (`�76` editing slice)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::{
        unified_diff, Applier, CreateFileTool, DeleteFileTool, EditOperation, EditStaging,
        ProposeEditTool, ProposedEdit, StagingFailure, TransactionState,
    };
    use crate::message::AiMessage;
    use crate::permission::{AiPermission, AiPermissionFailure, AiPermissionSet};
    use crate::session::AiSessionId;
    use crate::tools::{
        ApprovalGate, ReadFileTool, ToolContext, ToolFailure, ToolRegistry, WorkspaceSnapshot,
    };
    use nexus_core::ids::{CubeId, UserId};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    // -- test infrastructure ------------------------------------------------

    #[derive(Debug, Default, Clone)]
    struct MapStore(BTreeMap<String, String>);

    impl MapStore {
        fn new(entries: &[(&str, &str)]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(path, body)| (String::from(*path), String::from(*body)))
                    .collect(),
            )
        }
    }

    impl WorkspaceSnapshot for MapStore {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }

        fn list(&self) -> Vec<String> {
            self.0.keys().cloned().collect()
        }
    }

    /// Applier over a shared map; can fail a chosen path and record every
    /// effect so tests can trace rollback.
    #[derive(Debug, Default, Clone)]
    struct VecApplier {
        state: Arc<Mutex<BTreeMap<String, String>>>,
        effects: Arc<Mutex<Vec<String>>>,
        fail_write_on: Arc<Mutex<Option<String>>>,
    }

    impl VecApplier {
        fn new(entries: &[(&str, &str)]) -> Self {
            Self {
                state: Arc::new(Mutex::new(MapStore::new(entries).0)),
                ..Self::default()
            }
        }

        fn fail_write_on(&self, path: &str) {
            *self.fail_write_on.lock().unwrap() = Some(String::from(path));
        }
    }

    impl Applier for VecApplier {
        fn read(&self, path: &str) -> Result<String, String> {
            self.state
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| format!("no such file: {path}"))
        }

        fn write(&self, path: &str, content: &str) -> Result<(), String> {
            if *self.fail_write_on.lock().unwrap() == Some(String::from(path)) {
                return Err(format!("simulated write failure for {path}"));
            }
            self.state
                .lock()
                .unwrap()
                .insert(String::from(path), String::from(content));
            self.effects.lock().unwrap().push(format!("write {path}"));
            Ok(())
        }

        fn delete(&self, path: &str) -> Result<(), String> {
            if self.state.lock().unwrap().remove(path).is_none() {
                return Err(format!("no such file: {path}"));
            }
            self.effects.lock().unwrap().push(format!("delete {path}"));
            Ok(())
        }

        fn rename(&self, _from: &str, _to: &str) -> Result<(), String> {
            Err(String::from("renames are not wired in tests"))
        }
    }

    struct ApprovingGate;

    impl ApprovalGate for ApprovingGate {
        fn approved(&self, _user: UserId, _permission: AiPermission) -> bool {
            true
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            session: AiSessionId::new(1),
            user: UserId::from_u64(1),
            cube: CubeId::from_u64(7),
        }
    }

    fn workspace_of(applier: &VecApplier) -> Arc<dyn WorkspaceSnapshot> {
        Arc::new(MapStore(applier.state.lock().unwrap().clone()))
    }

    const ORDER: &str = "fn calculate_total(s: &[u32]) -> u32 {\n    s.iter().sum()\n}\n";
    const CONSTANT: &str = "pub const TAX_RATE: u32 = 19;\n";

    // -- diff generation -----------------------------------------------------

    #[test]
    fn unified_diff_generates_marked_hunks() {
        let old = "a\nb\nc\n";
        let new = "a\nx\nc\nd\n";
        let diff = unified_diff(old, new);
        assert_eq!(diff, " a\n-b\n+x\n c\n+d");
    }

    #[test]
    fn unified_diff_fully_new_file_is_only_additions() {
        let diff = unified_diff("", "one\ntwo\n");
        assert_eq!(diff, "+one\n+two");
    }

    // -- proposal validation -------------------------------------------------

    #[tokio::test]
    async fn propose_edit_rejects_missing_or_ambiguous_find() {
        let applier = VecApplier::new(&[("src/order.rs", ORDER)]);
        let staging = super::shared_staging();
        let mut registry = ToolRegistry::new(
            Arc::new(|_| {
                AiPermissionSet::new([
                    AiPermission::ReadWorkspace,
                    AiPermission::ProposeEdit,
                    AiPermission::CreateFiles,
                ])
            }),
            Arc::new(()),
        );
        registry.register(Arc::new(ProposeEditTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));

        // `find` never occurs.
        let miss = registry
            .call(
                "propose_edit",
                &context(),
                r#"{"path":"src/order.rs","find":"NOPE","replace":"y"}"#,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(miss, ToolFailure::ToolError(message) if message.contains("exactly once"))
        );
        // `find` occurs twice.
        let ambiguous = registry
            .call(
                "propose_edit",
                &context(),
                r#"{"path":"src/order.rs","find":"u32","replace":"u64"}"#,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(ambiguous, ToolFailure::ToolError(message) if message.contains("found 2 times"))
        );
        // Missing file.
        let absent = registry
            .call(
                "propose_edit",
                &context(),
                r#"{"path":"src/main.rs","find":"a","replace":"b"}"#,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(absent, ToolFailure::ToolError(message) if message.contains("no such file"))
        );
    }

    #[tokio::test]
    async fn propose_edit_stages_diff_with_base_capture() {
        let applier = VecApplier::new(&[("src/order.rs", ORDER)]);
        let staging = super::shared_staging();
        let mut registry = ToolRegistry::new(
            Arc::new(|_| AiPermissionSet::new([AiPermission::ProposeEdit])),
            Arc::new(()),
        );
        registry.register(Arc::new(ProposeEditTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));

        let result = registry
            .call(
                "propose_edit",
                &context(),
                r#"{"path":"src/order.rs","find":"sum()","replace":"sum::<u32>()"}"#,
            )
            .await
            .unwrap();
        assert!(result.output.starts_with("transaction 0 staged"));
        assert!(result.output.contains('-'));
        assert!(result.output.contains("+    s.iter().sum::<u32>()"));
        // The proposal is not applied � workspace unchanged.
        assert_eq!(applier.read("src/order.rs").unwrap(), ORDER);
        // Staging recorded the base for conflict detection.
        let staged = staging.lock().unwrap().get(0).unwrap().clone();
        assert_eq!(staged.state, TransactionState::Proposed);
        assert_eq!(staged.edits.len(), 1);
        assert_eq!(staged.edits[0].base.as_deref(), Some(ORDER));
        assert_eq!(staged.edits[0].operation, EditOperation::Write);
    }

    #[tokio::test]
    async fn create_file_stages_new_file_proposal() {
        let applier = VecApplier::new(&[("src/order.rs", ORDER)]);
        let staging = super::shared_staging();
        let mut registry = ToolRegistry::new(
            Arc::new(|_| AiPermissionSet::new([AiPermission::CreateFiles])),
            Arc::new(()),
        );
        registry.register(Arc::new(CreateFileTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));

        let staged_file = registry
            .call(
                "create_file",
                &context(),
                r#"{"path":"src/tests.rs","content":"mod order;\n"}"#,
            )
            .await
            .unwrap();
        assert!(staged_file.output.contains("new file src/tests.rs"));
        // Existing paths are refused.
        assert!(registry
            .call(
                "create_file",
                &context(),
                r#"{"path":"src/order.rs","content":"x"}"#
            )
            .await
            .is_err());
    }

    // -- approval gating ------------------------------------------------------

    #[tokio::test]
    async fn delete_file_requires_approval_at_the_registry_gate() {
        let applier = VecApplier::new(&[("src/order.rs", ORDER), ("old.rs", CONSTANT)]);
        let staging = super::shared_staging();
        let mut denied = ToolRegistry::new(
            Arc::new(|_| AiPermissionSet::new([AiPermission::DeleteFiles])),
            Arc::new(()),
        );
        denied.register(Arc::new(DeleteFileTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));
        let refusal = denied
            .call("delete_file", &context(), r#"{"path":"old.rs"}"#)
            .await
            .unwrap_err();
        assert_eq!(
            refusal,
            ToolFailure::Permission(AiPermissionFailure::NeedsApproval(
                AiPermission::DeleteFiles
            ))
        );

        let mut approving = ToolRegistry::new(
            Arc::new(|_| AiPermissionSet::new([AiPermission::DeleteFiles])),
            Arc::new(ApprovingGate),
        );
        approving.register(Arc::new(DeleteFileTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));
        let accepted = approving
            .call("delete_file", &context(), r#"{"path":"old.rs"}"#)
            .await
            .unwrap();
        assert!(accepted.output.contains("delete old.rs"));
        assert_eq!(
            staging.lock().unwrap().get(0).unwrap().edits[0].operation,
            EditOperation::Delete
        );
        assert!(staging.lock().unwrap().get(0).unwrap().needs_approval());
    }

    // -- apply / rollback / undo ---------------------------------------------

    #[test]
    fn apply_flow_propose_approve_apply_and_undo() {
        let mut staging = EditStaging::new();
        let applier = VecApplier::new(&[("src/order.rs", ORDER), ("README", CONSTANT)]);
        let id = staging.propose(
            "fix",
            vec![ProposedEdit {
                path: String::from("src/order.rs"),
                base: Some(String::from(ORDER)),
                new_text: String::from("replaced body\n"),
                operation: EditOperation::Write,
            }],
        );

        // Preview is available before approval.
        assert_eq!(staging.get(id).unwrap().name, "fix");

        assert!(staging.apply(id, &applier).is_err()); // not approved yet
        staging.approve(id).unwrap();
        let applied = staging.apply(id, &applier).unwrap();
        assert_eq!(applied.written, vec![String::from("src/order.rs")]);
        assert_eq!(applier.read("src/order.rs").unwrap(), "replaced body\n");
        assert_eq!(staging.get(id).unwrap().state, TransactionState::Applied);

        // Undo restores the pre-apply content.
        staging.rollback(id, &applier).unwrap();
        assert_eq!(applier.read("src/order.rs").unwrap(), ORDER);
        assert_eq!(staging.get(id).unwrap().state, TransactionState::RolledBack);
    }

    #[test]
    fn stale_base_conflict_blocks_apply() {
        let mut staging = EditStaging::new();
        let applier = VecApplier::new(&[("src/order.rs", ORDER)]);
        let id = staging.propose(
            "fix",
            vec![ProposedEdit {
                path: String::from("src/order.rs"),
                base: Some(String::from("the user edited this since\n")),
                new_text: String::from("conflicting text\n"),
                operation: EditOperation::Write,
            }],
        );
        staging.approve(id).unwrap();
        let failure = staging.apply(id, &applier).unwrap_err();
        assert_eq!(
            failure,
            StagingFailure::StaleBase(String::from("src/order.rs"))
        );
        // Nothing moved.
        assert_eq!(applier.read("src/order.rs").unwrap(), ORDER);
        assert_eq!(staging.get(id).unwrap().state, TransactionState::Approved);
    }

    #[test]
    fn mid_apply_failure_rolls_back_already_applied_effects() {
        let mut staging = EditStaging::new();
        let applier = VecApplier::new(&[("a.rs", "one\n"), ("b.rs", "two\n")]);
        applier.fail_write_on("b.rs");
        let id = staging.propose(
            "cross-file",
            vec![
                ProposedEdit {
                    path: String::from("a.rs"),
                    base: Some(String::from("one\n")),
                    new_text: String::from("one changed\n"),
                    operation: EditOperation::Write,
                },
                ProposedEdit {
                    path: String::from("b.rs"),
                    base: Some(String::from("two\n")),
                    new_text: String::from("two changed\n"),
                    operation: EditOperation::Write,
                },
            ],
        );
        staging.approve(id).unwrap();
        let failure = staging.apply(id, &applier).unwrap_err();
        assert!(
            matches!(failure, StagingFailure::WorkspaceError(message) if message.contains("rolled back"))
        );
        assert_eq!(staging.get(id).unwrap().state, TransactionState::Failed);
        // No partial workspace: both files restored.
        assert_eq!(applier.read("a.rs").unwrap(), "one\n");
        assert_eq!(applier.read("b.rs").unwrap(), "two\n");
    }

    #[test]
    fn rollback_restores_deleted_and_created_files() {
        let mut staging = EditStaging::new();
        let applier = VecApplier::new(&[("src/order.rs", ORDER)]);
        let id = staging.propose(
            "sweep",
            vec![
                ProposedEdit {
                    path: String::from("src/order.rs"),
                    base: Some(String::from(ORDER)),
                    new_text: String::new(),
                    operation: EditOperation::Delete,
                },
                ProposedEdit {
                    path: String::from("src/new.rs"),
                    base: None,
                    new_text: String::from("brand new\n"),
                    operation: EditOperation::Create,
                },
            ],
        );
        staging.approve(id).unwrap();
        let applied = staging.apply(id, &applier).unwrap();
        assert_eq!(applied.deleted, vec![String::from("src/order.rs")]);
        assert_eq!(applied.written, vec![String::from("src/new.rs")]);
        assert!(applier.read("src/order.rs").is_err());
        assert_eq!(applier.read("src/new.rs").unwrap(), "brand new\n");

        staging.rollback(id, &applier).unwrap();
        assert_eq!(applier.read("src/order.rs").unwrap(), ORDER);
        assert!(applier.read("src/new.rs").is_err());
    }

    // -- end-to-end loop through the registry ---------------------------------

    #[tokio::test]
    async fn registry_loop_propose_then_host_applies_then_model_reads_back() {
        let applier = VecApplier::new(&[("src/order.rs", ORDER), ("src/main.rs", CONSTANT)]);
        let staging = super::shared_staging();
        let mut registry = ToolRegistry::new(
            Arc::new(|_| {
                AiPermissionSet::new([AiPermission::ReadWorkspace, AiPermission::ProposeEdit])
            }),
            Arc::new(()),
        );
        registry.register(Arc::new(ReadFileTool::new(workspace_of(&applier))));
        registry.register(Arc::new(ProposeEditTool::new(
            workspace_of(&applier),
            staging.clone(),
        )));

        // 1) The model reads the file first (context grounding).
        let seen = registry
            .call("read_file", &context(), r#"{"path":"src/order.rs"}"#)
            .await
            .unwrap();
        assert!(seen.output.contains("calculate_total"));

        // 2) The model proposes an edit through the diff-first tool.
        let proposal = registry
            .call(
                "propose_edit",
                &context(),
                r#"{"path":"src/order.rs","find":"sum()","replace":"sum::<u32>()"}"#,
            )
            .await
            .unwrap();
        let id = proposal
            .output
            .strip_prefix("transaction ")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|digits| digits.parse::<u64>().ok())
            .unwrap();

        // 3) The human approves; the host applies through the sync boundary.
        staging.lock().unwrap().approve(id).unwrap();
        let applied = staging.lock().unwrap().apply(id, &applier).unwrap();
        assert_eq!(applied.written, vec![String::from("src/order.rs")]);
        assert_eq!(
            applier.read("src/order.rs").unwrap(),
            "fn calculate_total(s: &[u32]) -> u32 {\n    s.iter().sum::<u32>()\n}\n"
        );

        // 4) Sanity: history turn builder accepts the resulting text.
        let turn = AiMessage::tool_result("call-1", proposal.output.clone());
        assert_eq!(turn.role, crate::message::AiRole::ToolResult);
    }
}

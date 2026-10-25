//! Deterministic governance for AegisFlow.
//!
//! * [`GovernanceEngine`] — `cedar-policy` 4.x authorizer over `policies/aegis_rules.cedar`,
//!   validated in strict mode against `policies/aegis.cedarschema` when loaded.
//! * [`redact`] — regex detection and `[REDACTED]` formatting of sensitive strings.
//! * [`receipt`] — Ed25519 signatures over the SHA-256 digest of canonical JSON, encoded as
//!   `c2pa:ed25519:<base64url>` (a label; these are not C2PA manifests).

pub mod receipt;
pub mod redact;
pub mod time;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use cedar_policy::{
    Authorizer, Context, Decision as CedarDecision, Entities, EntityId, EntityTypeName, EntityUid, PolicyId,
    PolicySet, Request, RestrictedExpression, Schema, ValidationMode, Validator,
};

pub use receipt::{Receipt, ReceiptError, ReceiptSigner};
pub use redact::{scanner, ScanReport, SensitiveScanner};

/// Risk score at or above which an operation is high impact (mirrors the Cedar rules).
pub const HIGH_IMPACT_THRESHOLD: i64 = 35;

#[derive(Debug, thiserror::Error)]
pub enum GovernanceError {
    #[error("reading {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("policy parse error: {0}")]
    PolicyParse(String),
    #[error("schema parse error: {0}")]
    SchemaParse(String),
    #[error("policies do not validate against the schema: {0}")]
    Validation(String),
    #[error("invalid request: {0}")]
    Request(String),
}

/// Actions declared in `policies/aegis.cedarschema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Branch,
    Rollback,
    SpeculativeCommit,
    CreateOrder,
}

impl Action {
    pub fn cedar_id(self) -> &'static str {
        match self {
            Action::Branch => "workspace::branch",
            Action::Rollback => "workspace::rollback",
            Action::SpeculativeCommit => "workspace::speculative_commit",
            Action::CreateOrder => "paypal::create_order",
        }
    }

    /// Entity type of the resource this action applies to.
    pub fn resource_type(self) -> &'static str {
        match self {
            Action::CreateOrder => "Merchant",
            _ => "Workspace",
        }
    }

    pub fn parse(name: &str) -> Option<Action> {
        [Action::Branch, Action::Rollback, Action::SpeculativeCommit, Action::CreateOrder]
            .into_iter()
            .find(|a| a.cedar_id() == name)
    }
}

/// Request context; field names match the schema's `AegisContext`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyContext {
    pub ane_risk_score: i64,
    pub has_unformatted_sensitive_fields: bool,
    pub team_approval_granted: bool,
    pub ed25519_receipt_verified: bool,
    pub amount_cents: i64,
}

#[derive(Debug, Clone)]
pub struct AuthzRequest {
    pub principal: String,
    pub action: Action,
    pub resource: String,
    pub context: PolicyContext,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Decision {
    pub allowed: bool,
    /// `@id` annotations of the policies that determined the decision (empty = default deny).
    pub determining_policies: Vec<String>,
    pub errors: Vec<String>,
    /// Wall time of `Authorizer::is_authorized`, nanoseconds.
    pub eval_ns: u64,
}

pub struct GovernanceEngine {
    policies: PolicySet,
    schema: Schema,
    authorizer: Authorizer,
    entities: Entities,
    names: HashMap<PolicyId, String>,
}

impl GovernanceEngine {
    /// `policies/aegis_rules.cedar` and `policies/aegis.cedarschema` of this repository, unless
    /// `AEGIS_POLICY_DIR` points elsewhere.
    pub fn default_paths() -> (PathBuf, PathBuf) {
        let dir = std::env::var_os("AEGIS_POLICY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policies"));
        (dir.join("aegis_rules.cedar"), dir.join("aegis.cedarschema"))
    }

    pub fn load_default() -> Result<Self, GovernanceError> {
        let (policies, schema) = Self::default_paths();
        Self::from_files(&policies, &schema)
    }

    pub fn from_files(policy_path: &Path, schema_path: &Path) -> Result<Self, GovernanceError> {
        let read = |p: &Path| {
            std::fs::read_to_string(p).map_err(|source| GovernanceError::Read { path: p.to_path_buf(), source })
        };
        Self::from_sources(&read(policy_path)?, &read(schema_path)?)
    }

    /// Parses policies and schema and validates the policies in strict mode.
    pub fn from_sources(policy_src: &str, schema_src: &str) -> Result<Self, GovernanceError> {
        let (schema, _warnings) =
            Schema::from_cedarschema_str(schema_src).map_err(|e| GovernanceError::SchemaParse(e.to_string()))?;
        let policies = Self::parse_and_validate(policy_src, &schema)?;
        let names = policies
            .policies()
            .map(|p| (p.id().clone(), p.annotation("id").map(str::to_owned).unwrap_or_else(|| p.id().to_string())))
            .collect();
        Ok(Self { policies, schema, authorizer: Authorizer::new(), entities: Entities::empty(), names })
    }

    fn parse_and_validate(policy_src: &str, schema: &Schema) -> Result<PolicySet, GovernanceError> {
        let policies = PolicySet::from_str(policy_src).map_err(|e| GovernanceError::PolicyParse(e.to_string()))?;
        let result = Validator::new(schema.clone()).validate(&policies, ValidationMode::Strict);
        if !result.validation_passed() {
            let errors: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
            return Err(GovernanceError::Validation(errors.join("; ")));
        }
        Ok(policies)
    }

    /// Checks a proposed replacement policy file (e.g. from the build-recovery router) against
    /// the loaded schema. Returns the number of policies it contains.
    pub fn validate_policy_text(&self, policy_src: &str) -> Result<usize, GovernanceError> {
        Ok(Self::parse_and_validate(policy_src, &self.schema)?.policies().count())
    }

    pub fn policy_count(&self) -> usize {
        self.policies.policies().count()
    }

    pub fn policy_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.names.values().cloned().collect();
        names.sort();
        names
    }

    /// Builds a schema-validated Cedar request.
    pub fn build_request(&self, req: &AuthzRequest) -> Result<Request, GovernanceError> {
        let uid = |ty: &str, id: &str| -> Result<EntityUid, GovernanceError> {
            let ty = EntityTypeName::from_str(ty).map_err(|e| GovernanceError::Request(e.to_string()))?;
            Ok(EntityUid::from_type_name_and_id(ty, EntityId::new(id)))
        };
        let ctx = &req.context;
        let context = Context::from_pairs([
            ("ane_risk_score".to_string(), RestrictedExpression::new_long(ctx.ane_risk_score)),
            (
                "has_unformatted_sensitive_fields".to_string(),
                RestrictedExpression::new_bool(ctx.has_unformatted_sensitive_fields),
            ),
            ("team_approval_granted".to_string(), RestrictedExpression::new_bool(ctx.team_approval_granted)),
            ("ed25519_receipt_verified".to_string(), RestrictedExpression::new_bool(ctx.ed25519_receipt_verified)),
            ("amount_cents".to_string(), RestrictedExpression::new_long(ctx.amount_cents)),
        ])
        .map_err(|e| GovernanceError::Request(e.to_string()))?;
        Request::new(
            uid("Agent", &req.principal)?,
            uid("Action", req.action.cedar_id())?,
            uid(req.action.resource_type(), &req.resource)?,
            context,
            Some(&self.schema),
        )
        .map_err(|e| GovernanceError::Request(e.to_string()))
    }

    /// Evaluates a prebuilt request.
    pub fn authorize_request(&self, request: &Request) -> Decision {
        let started = Instant::now();
        let response = self.authorizer.is_authorized(request, &self.policies, &self.entities);
        let eval_ns = started.elapsed().as_nanos() as u64;
        let mut determining: Vec<String> = response
            .diagnostics()
            .reason()
            .map(|id| self.names.get(id).cloned().unwrap_or_else(|| id.to_string()))
            .collect();
        determining.sort();
        Decision {
            allowed: response.decision() == CedarDecision::Allow,
            determining_policies: determining,
            errors: response.diagnostics().errors().map(|e| e.to_string()).collect(),
            eval_ns,
        }
    }

    pub fn authorize(&self, req: &AuthzRequest) -> Result<Decision, GovernanceError> {
        Ok(self.authorize_request(&self.build_request(req)?))
    }
}

/// Clamps a 0..100 model score to the integer Cedar context value.
pub fn risk_to_context(score: f32) -> i64 {
    score.round().clamp(0.0, 100.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> GovernanceEngine {
        GovernanceEngine::load_default().expect("policies must parse and validate")
    }

    fn ask(engine: &GovernanceEngine, action: Action, ctx: PolicyContext) -> Decision {
        let resource = if action == Action::CreateOrder { "sandbox-merchant" } else { "aegisflow" };
        engine
            .authorize(&AuthzRequest { principal: "core-builder".into(), action, resource: resource.into(), context: ctx })
            .unwrap()
    }

    #[test]
    fn policies_validate_in_strict_mode() {
        let e = engine();
        assert_eq!(e.policy_count(), 5);
        assert!(e.policy_names().contains(&"forbid-order-over-50-usd-without-receipt".to_string()));
    }

    #[test]
    fn low_risk_commit_is_permitted() {
        let d = ask(&engine(), Action::SpeculativeCommit, PolicyContext { ane_risk_score: 12, ..Default::default() });
        assert!(d.allowed);
        assert_eq!(d.determining_policies, vec!["permit-low-risk"]);
    }

    #[test]
    fn sensitive_fields_block_low_risk_commit() {
        let ctx = PolicyContext { ane_risk_score: 5, has_unformatted_sensitive_fields: true, ..Default::default() };
        let d = ask(&engine(), Action::SpeculativeCommit, ctx);
        assert!(!d.allowed);
        assert!(d.determining_policies.is_empty(), "default deny");
    }

    #[test]
    fn high_impact_commit_needs_team_approval() {
        let e = engine();
        let ctx = PolicyContext { ane_risk_score: 35, ..Default::default() };
        let denied = ask(&e, Action::SpeculativeCommit, ctx.clone());
        assert!(!denied.allowed);
        assert_eq!(denied.determining_policies, vec!["forbid-high-impact-without-team-approval"]);
        let approved = ask(&e, Action::SpeculativeCommit, PolicyContext { team_approval_granted: true, ..ctx });
        assert!(approved.allowed);
        assert_eq!(approved.determining_policies, vec!["permit-approved-high-impact-commit"]);
    }

    #[test]
    fn orders_over_fifty_dollars_need_verified_receipt() {
        let e = engine();
        let base = PolicyContext { ane_risk_score: 20, amount_cents: 5001, ..Default::default() };
        let denied = ask(&e, Action::CreateOrder, base.clone());
        assert!(!denied.allowed);
        assert_eq!(denied.determining_policies, vec!["forbid-order-over-50-usd-without-receipt"]);
        let allowed = ask(&e, Action::CreateOrder, PolicyContext { ed25519_receipt_verified: true, ..base });
        assert!(allowed.allowed);
    }

    #[test]
    fn order_of_exactly_fifty_dollars_is_standard() {
        let ctx = PolicyContext { ane_risk_score: 20, amount_cents: 5000, ..Default::default() };
        assert!(ask(&engine(), Action::CreateOrder, ctx).allowed);
    }

    #[test]
    fn high_risk_order_is_denied_even_with_receipt() {
        let ctx = PolicyContext {
            ane_risk_score: 80,
            amount_cents: 2000,
            ed25519_receipt_verified: true,
            team_approval_granted: true,
            ..Default::default()
        };
        assert!(!ask(&engine(), Action::CreateOrder, ctx).allowed);
    }

    #[test]
    fn branch_and_rollback_always_allowed() {
        let e = engine();
        let risky = PolicyContext { ane_risk_score: 99, has_unformatted_sensitive_fields: true, ..Default::default() };
        assert!(ask(&e, Action::Branch, risky.clone()).allowed);
        assert!(ask(&e, Action::Rollback, risky).allowed);
    }

    #[test]
    fn invalid_policy_update_is_rejected() {
        let e = engine();
        let typo = r#"permit (principal, action == Action::"workspace::speculative_commit", resource) when { context.ane_risk_scor < 10 };"#;
        assert!(matches!(e.validate_policy_text(typo), Err(GovernanceError::Validation(_))));
        let ok = r#"permit (principal is Agent, action == Action::"workspace::branch", resource is Workspace);"#;
        assert_eq!(e.validate_policy_text(ok).unwrap(), 1);
    }

    #[test]
    fn risk_scores_are_clamped() {
        assert_eq!(risk_to_context(34.4), 34);
        assert_eq!(risk_to_context(34.6), 35);
        assert_eq!(risk_to_context(140.0), 100);
        assert_eq!(risk_to_context(-3.0), 0);
    }
}

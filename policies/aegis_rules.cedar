// AegisFlow governance rules, evaluated by crates/aegis-governance (cedar-policy 4.x).
// Schema: policies/aegis.cedarschema. Cedar semantics: any matching forbid overrides every
// permit, and a request that matches no permit is denied.

// Branching and rolling back never touch the live workspace.
@id("permit-branch-and-rollback")
permit (
  principal is Agent,
  action in [Action::"workspace::branch", Action::"workspace::rollback"],
  resource is Workspace
);

// Speculative workspace commits and standard orders: low ANE risk, no unredacted sensitive data.
@id("permit-low-risk")
permit (
  principal is Agent,
  action in [Action::"workspace::speculative_commit", Action::"paypal::create_order"],
  resource
)
when {
  context.ane_risk_score < 35 &&
  context.has_unformatted_sensitive_fields == false
};

// High-impact commits may proceed once a teammate approved them.
@id("permit-approved-high-impact-commit")
permit (
  principal is Agent,
  action == Action::"workspace::speculative_commit",
  resource is Workspace
)
when {
  context.ane_risk_score >= 35 &&
  context.team_approval_granted == true &&
  context.has_unformatted_sensitive_fields == false
};

// Orders above $50.00 need a verified ed25519 receipt.
@id("forbid-order-over-50-usd-without-receipt")
forbid (
  principal,
  action == Action::"paypal::create_order",
  resource
)
when { context.amount_cents > 5000 }
unless { context.ed25519_receipt_verified == true };

// High-impact workspace commits need team approval.
@id("forbid-high-impact-without-team-approval")
forbid (
  principal,
  action == Action::"workspace::speculative_commit",
  resource
)
when { context.ane_risk_score >= 35 }
unless { context.team_approval_granted == true };

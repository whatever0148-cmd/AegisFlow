//! The 64-feature operation vector consumed by the CoreML model.
//!
//! Names, order and normalisation must match `models/feature_schema.json`; a unit test enforces it.

use std::path::Path;
use std::sync::OnceLock;

use aegis_governance::scanner;
use aegis_storage::{ChangeKind, ChangeSet};
use regex::Regex;

pub const FEATURE_COUNT: usize = 64;

/// How a raw measurement is normalised into [0, 1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Binary,
    Ratio,
    /// `min(x / cap, 1)`
    Count(f64),
    /// `ln(1 + x) / ln(1 + cap)`, clipped
    Log(f64),
}

impl Kind {
    pub fn normalize(self, raw: f64) -> f32 {
        let raw = if raw.is_finite() { raw.max(0.0) } else { 0.0 };
        let v = match self {
            Kind::Binary => {
                if raw > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Ratio => raw,
            Kind::Count(cap) => raw / cap,
            Kind::Log(cap) => (1.0 + raw).ln() / (1.0 + cap).ln(),
        };
        v.clamp(0.0, 1.0) as f32
    }
}

macro_rules! features {
    ($($variant:ident => $name:literal, $kind:expr;)*) => {
        /// Feature slots in model input order.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum Feature { $($variant),* }

        pub const FEATURE_NAMES: [&str; FEATURE_COUNT] = [$($name),*];
        pub const FEATURE_KINDS: [Kind; FEATURE_COUNT] = [$($kind),*];

        impl Feature {
            pub const ALL: [Feature; FEATURE_COUNT] = [$(Feature::$variant),*];

            pub fn name(self) -> &'static str {
                FEATURE_NAMES[self as usize]
            }
        }
    };
}

features! {
    FilesTouched => "files_touched", Kind::Count(64.0);
    FilesAdded => "files_added", Kind::Count(32.0);
    FilesDeleted => "files_deleted", Kind::Count(32.0);
    BytesChanged => "bytes_changed", Kind::Log(1_048_576.0);
    LinesAdded => "lines_added", Kind::Log(5000.0);
    LinesRemoved => "lines_removed", Kind::Log(5000.0);
    MaxPathDepth => "max_path_depth", Kind::Count(12.0);
    BinaryFilesRatio => "binary_files_ratio", Kind::Ratio;
    LargeFileFlag => "large_file_flag", Kind::Binary;
    SymlinkChanges => "symlink_changes", Kind::Binary;
    PermissionChanges => "permission_changes", Kind::Binary;
    GeneratedFilesRatio => "generated_files_ratio", Kind::Ratio;
    BuildScriptTouched => "build_script_touched", Kind::Binary;
    LockfileTouched => "lockfile_touched", Kind::Binary;
    DependencyManifestTouched => "dependency_manifest_touched", Kind::Binary;
    CiConfigTouched => "ci_config_touched", Kind::Binary;
    InfraConfigTouched => "infra_config_touched", Kind::Binary;
    PolicyFileTouched => "policy_file_touched", Kind::Binary;
    MigrationTouched => "migration_touched", Kind::Binary;
    AuthCodeTouched => "auth_code_touched", Kind::Binary;
    CryptoCodeTouched => "crypto_code_touched", Kind::Binary;
    PaymentCodeTouched => "payment_code_touched", Kind::Binary;
    EnvFileTouched => "env_file_touched", Kind::Binary;
    VcsInternalTouched => "vcs_internal_touched", Kind::Binary;
    SecretPatternHits => "secret_pattern_hits", Kind::Count(4.0);
    EmailHits => "email_hits", Kind::Count(8.0);
    CardNumberHits => "card_number_hits", Kind::Count(2.0);
    SsnHits => "ssn_hits", Kind::Count(2.0);
    PhoneHits => "phone_hits", Kind::Count(8.0);
    UnsafeBlockDelta => "unsafe_block_delta", Kind::Count(8.0);
    ShellExecDelta => "shell_exec_delta", Kind::Count(8.0);
    NetworkEndpointDelta => "network_endpoint_delta", Kind::Count(8.0);
    StubMarkerDelta => "stub_marker_delta", Kind::Count(8.0);
    TestFilesRatio => "test_files_ratio", Kind::Ratio;
    DocsFilesRatio => "docs_files_ratio", Kind::Ratio;
    UnformattedSensitiveFields => "unformatted_sensitive_fields", Kind::Binary;
    BuildFailed => "build_failed", Kind::Binary;
    TestsFailedRatio => "tests_failed_ratio", Kind::Ratio;
    RetryCount => "retry_count", Kind::Count(5.0);
    AgentUncertainty => "agent_uncertainty", Kind::Ratio;
    CompileErrors => "compile_errors", Kind::Log(200.0);
    CompileWarnings => "compile_warnings", Kind::Log(500.0);
    BranchAgeMinutes => "branch_age_minutes", Kind::Log(1440.0);
    ConcurrentBranches => "concurrent_branches", Kind::Count(8.0);
    HoursSinceHumanReview => "hours_since_human_review", Kind::Log(168.0);
    OffHours => "off_hours", Kind::Binary;
    EscalationTier => "escalation_tier", Kind::Count(2.0);
    RollbacksLast24h => "rollbacks_last_24h", Kind::Count(10.0);
    OrderAmountCents => "order_amount_cents", Kind::Log(1_000_000.0);
    BudgetUtilization => "budget_utilization", Kind::Ratio;
    CurrencyNonUsd => "currency_non_usd", Kind::Binary;
    NewPayee => "new_payee", Kind::Binary;
    PayeeCountryRisk => "payee_country_risk", Kind::Ratio;
    LineItems => "line_items", Kind::Count(20.0);
    QuantityTotal => "quantity_total", Kind::Log(1000.0);
    ReceiptUnverified => "receipt_unverified", Kind::Binary;
    OrdersLastHour => "orders_last_hour", Kind::Count(20.0);
    SpendLast24hCents => "spend_last_24h_cents", Kind::Log(10_000_000.0);
    MerchantRefundRate => "merchant_refund_rate", Kind::Ratio;
    ShippingMismatch => "shipping_mismatch", Kind::Binary;
    CustomerPiiFields => "customer_pii_fields", Kind::Count(6.0);
    DiscountRatio => "discount_ratio", Kind::Ratio;
    DigitalGoods => "digital_goods", Kind::Binary;
    ManualOverrideRequested => "manual_override_requested", Kind::Binary;
}

/// Normalised model input. `repr(transparent)` keeps slices of vectors contiguous for batch FFI.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(transparent)]
pub struct FeatureVector(pub [f32; FEATURE_COUNT]);

impl Default for FeatureVector {
    fn default() -> Self {
        Self([0.0; FEATURE_COUNT])
    }
}

impl FeatureVector {
    /// Stores `raw` after normalising it with the feature's [`Kind`].
    pub fn put(&mut self, feature: Feature, raw: f64) {
        self.0[feature as usize] = FEATURE_KINDS[feature as usize].normalize(raw);
    }

    pub fn flag(&mut self, feature: Feature, on: bool) {
        self.put(feature, if on { 1.0 } else { 0.0 });
    }

    pub fn get(&self, feature: Feature) -> f32 {
        self.0[feature as usize]
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Non-zero features by name, for logs and approval cards.
    pub fn active(&self) -> Vec<(&'static str, f32)> {
        Feature::ALL.iter().filter(|f| self.get(**f) > 0.0).map(|f| (f.name(), self.get(*f))).collect()
    }
}

/// Signals about the agent run that produced a change.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    pub build_failed: bool,
    pub tests_failed_ratio: f64,
    pub retry_count: u32,
    /// 1 - the agent's self-reported confidence, in [0, 1].
    pub agent_uncertainty: f64,
    pub compile_errors: u32,
    pub compile_warnings: u32,
    pub branch_age_minutes: f64,
    pub concurrent_branches: u32,
    pub hours_since_human_review: f64,
    pub off_hours: bool,
    pub escalation_tier: u32,
    pub rollbacks_last_24h: u32,
}

impl ExecutionContext {
    fn apply(&self, v: &mut FeatureVector) {
        v.flag(Feature::BuildFailed, self.build_failed);
        v.put(Feature::TestsFailedRatio, self.tests_failed_ratio);
        v.put(Feature::RetryCount, f64::from(self.retry_count));
        v.put(Feature::AgentUncertainty, self.agent_uncertainty);
        v.put(Feature::CompileErrors, f64::from(self.compile_errors));
        v.put(Feature::CompileWarnings, f64::from(self.compile_warnings));
        v.put(Feature::BranchAgeMinutes, self.branch_age_minutes);
        v.put(Feature::ConcurrentBranches, f64::from(self.concurrent_branches));
        v.put(Feature::HoursSinceHumanReview, self.hours_since_human_review);
        v.flag(Feature::OffHours, self.off_hours);
        v.put(Feature::EscalationTier, f64::from(self.escalation_tier));
        v.put(Feature::RollbacksLast24h, f64::from(self.rollbacks_last_24h));
    }
}

/// Signals about an agent purchase.
#[derive(Debug, Clone, Default)]
pub struct CommerceContext {
    pub amount_cents: u64,
    pub budget_cents: u64,
    pub currency: String,
    pub new_payee: bool,
    pub payee_country_risk: f64,
    pub line_items: u32,
    pub quantity_total: u32,
    pub receipt_verified: bool,
    pub orders_last_hour: u32,
    pub spend_last_24h_cents: u64,
    pub merchant_refund_rate: f64,
    pub shipping_mismatch: bool,
    pub customer_pii_fields: u32,
    pub discount_ratio: f64,
    pub digital_goods: bool,
    pub manual_override_requested: bool,
    /// Sensitive values still present (unredacted) in the outbound payload.
    pub unformatted_sensitive_fields: bool,
}

struct PathFlags {
    build_script: bool,
    lockfile: bool,
    manifest: bool,
    ci: bool,
    infra: bool,
    policy: bool,
    migration: bool,
    auth: bool,
    crypto: bool,
    payment: bool,
    env: bool,
    vcs: bool,
    generated: bool,
    test: bool,
    docs: bool,
}

fn classify(path: &Path) -> PathFlags {
    let full = path.to_string_lossy().to_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower_name = name.to_lowercase();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let any = |needles: &[&str]| needles.iter().any(|n| full.contains(n));
    PathFlags {
        build_script: ["build.rs", "Makefile", "CMakeLists.txt", "build.gradle", "BUILD", "BUILD.bazel", "justfile"]
            .contains(&name.as_str())
            || ext == "mk",
        lockfile: ["Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "Podfile.lock", "poetry.lock", "Gemfile.lock", "go.sum", "uv.lock"]
            .contains(&name.as_str()),
        manifest: ["Cargo.toml", "package.json", "requirements.txt", "pyproject.toml", "go.mod", "Gemfile", "Podfile", "Package.swift"]
            .contains(&name.as_str()),
        ci: full.starts_with(".github/workflows/") || full.starts_with(".circleci/") || full.starts_with(".buildkite/")
            || lower_name == ".gitlab-ci.yml" || lower_name == "jenkinsfile",
        infra: lower_name.starts_with("dockerfile") || lower_name.starts_with("docker-compose") || ext == "tf"
            || any(&["k8s/", "helm/", "terraform/", "kubernetes/"]),
        policy: ext == "cedar" || ext == "cedarschema" || ext == "rego" || full.starts_with("policies/"),
        migration: full.contains("migrations/") || ext == "sql",
        auth: any(&["auth", "login", "session", "oauth", "permission"]),
        crypto: any(&["crypto", "signing", "keys/", "tls", "cert"]) || ext == "pem" || ext == "key",
        payment: any(&["payment", "billing", "checkout", "paypal", "stripe", "invoice"]),
        env: lower_name.starts_with(".env"),
        vcs: full.starts_with(".git/"),
        generated: any(&["/generated/", "dist/", "build/"]) || full.ends_with(".min.js") || full.ends_with(".pb.go")
            || full.ends_with("_pb2.py"),
        test: any(&["tests/", "test/", "__tests__/"]) || lower_name.starts_with("test_") || lower_name.ends_with("_test.rs")
            || lower_name.ends_with("_test.go") || lower_name.contains(".test.") || lower_name.contains(".spec."),
        docs: ext == "md" || ext == "rst" || ext == "adoc" || full.starts_with("docs/"),
    }
}

struct ContentPatterns {
    unsafe_code: Regex,
    shell_exec: Regex,
    network: Regex,
    stub_marker: Regex,
}

fn patterns() -> &'static ContentPatterns {
    static PATTERNS: OnceLock<ContentPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| ContentPatterns {
        unsafe_code: Regex::new(r"\bunsafe\s*(\{|fn\b|impl\b|extern\b)").expect("static regex"),
        shell_exec: Regex::new(r"Command::new|subprocess\.|os\.system|popen\(|\bexecv?p?\(|\bsystem\(").expect("static regex"),
        network: Regex::new(r"https?://").expect("static regex"),
        // Character classes keep these marker words out of this source file's own stub scan.
        stub_marker: Regex::new(r"\b(TO[D]O|FIX[M]E|HAC[K]|X{3})\b").expect("static regex"),
    })
}

/// Features describing a workspace change set plus its execution context.
pub fn workspace_features(changes: &ChangeSet, exec: &ExecutionContext) -> FeatureVector {
    let mut v = FeatureVector::default();
    let n = changes.files.len();
    let ratio = |count: usize| if n == 0 { 0.0 } else { count as f64 / n as f64 };

    v.put(Feature::FilesTouched, n as f64);
    v.put(Feature::FilesAdded, changes.count(ChangeKind::Added) as f64);
    v.put(Feature::FilesDeleted, changes.count(ChangeKind::Deleted) as f64);
    v.put(Feature::BytesChanged, changes.bytes_changed() as f64);
    v.put(Feature::LinesAdded, changes.lines_added() as f64);
    v.put(Feature::LinesRemoved, changes.lines_removed() as f64);
    v.put(Feature::MaxPathDepth, changes.files.iter().map(|f| f.path.components().count()).max().unwrap_or(0) as f64);
    v.put(Feature::BinaryFilesRatio, ratio(changes.files.iter().filter(|f| f.binary).count()));
    v.flag(Feature::LargeFileFlag, changes.files.iter().any(|f| f.new_len.max(f.old_len) > 1_048_576));
    v.flag(Feature::SymlinkChanges, changes.files.iter().any(|f| f.is_symlink));
    v.flag(Feature::PermissionChanges, changes.files.iter().any(|f| f.mode_changed));

    let flags: Vec<PathFlags> = changes.files.iter().map(|f| classify(&f.path)).collect();
    let has = |pick: fn(&PathFlags) -> bool| flags.iter().any(pick);
    v.put(Feature::GeneratedFilesRatio, ratio(flags.iter().filter(|f| f.generated).count()));
    v.flag(Feature::BuildScriptTouched, has(|f| f.build_script));
    v.flag(Feature::LockfileTouched, has(|f| f.lockfile));
    v.flag(Feature::DependencyManifestTouched, has(|f| f.manifest));
    v.flag(Feature::CiConfigTouched, has(|f| f.ci));
    v.flag(Feature::InfraConfigTouched, has(|f| f.infra));
    v.flag(Feature::PolicyFileTouched, has(|f| f.policy));
    v.flag(Feature::MigrationTouched, has(|f| f.migration));
    v.flag(Feature::AuthCodeTouched, has(|f| f.auth));
    v.flag(Feature::CryptoCodeTouched, has(|f| f.crypto));
    v.flag(Feature::PaymentCodeTouched, has(|f| f.payment));
    v.flag(Feature::EnvFileTouched, has(|f| f.env));
    v.flag(Feature::VcsInternalTouched, has(|f| f.vcs));
    v.put(Feature::TestFilesRatio, ratio(flags.iter().filter(|f| f.test).count()));
    v.put(Feature::DocsFilesRatio, ratio(flags.iter().filter(|f| f.docs).count()));

    let p = patterns();
    let mut sensitive = aegis_governance::ScanReport::default();
    let (mut unsafe_hits, mut shell_hits, mut network_hits, mut stub_hits) = (0usize, 0usize, 0usize, 0usize);
    for change in &changes.files {
        let text = &change.added_text;
        if text.is_empty() {
            continue;
        }
        sensitive.merge(scanner().scan(text));
        if change.path.extension().is_some_and(|e| e == "rs") {
            unsafe_hits += p.unsafe_code.find_iter(text).count();
        }
        shell_hits += p.shell_exec.find_iter(text).count();
        network_hits += p.network.find_iter(text).count();
        stub_hits += p.stub_marker.find_iter(text).count();
    }
    v.put(Feature::SecretPatternHits, sensitive.secrets as f64);
    v.put(Feature::EmailHits, sensitive.emails as f64);
    v.put(Feature::CardNumberHits, sensitive.card_numbers as f64);
    v.put(Feature::SsnHits, sensitive.ssns as f64);
    v.put(Feature::PhoneHits, sensitive.phones as f64);
    v.put(Feature::UnsafeBlockDelta, unsafe_hits as f64);
    v.put(Feature::ShellExecDelta, shell_hits as f64);
    v.put(Feature::NetworkEndpointDelta, network_hits as f64);
    v.put(Feature::StubMarkerDelta, stub_hits as f64);
    v.flag(Feature::UnformattedSensitiveFields, sensitive.any());

    exec.apply(&mut v);
    v
}

/// Features describing an order (workspace features stay zero).
pub fn commerce_features(order: &CommerceContext, exec: Option<&ExecutionContext>) -> FeatureVector {
    let mut v = FeatureVector::default();
    v.put(Feature::OrderAmountCents, order.amount_cents as f64);
    let utilization = if order.budget_cents == 0 { 1.0 } else { order.amount_cents as f64 / order.budget_cents as f64 };
    v.put(Feature::BudgetUtilization, utilization);
    v.flag(Feature::CurrencyNonUsd, !order.currency.eq_ignore_ascii_case("USD"));
    v.flag(Feature::NewPayee, order.new_payee);
    v.put(Feature::PayeeCountryRisk, order.payee_country_risk);
    v.put(Feature::LineItems, f64::from(order.line_items));
    v.put(Feature::QuantityTotal, f64::from(order.quantity_total));
    v.flag(Feature::ReceiptUnverified, !order.receipt_verified);
    v.put(Feature::OrdersLastHour, f64::from(order.orders_last_hour));
    v.put(Feature::SpendLast24hCents, order.spend_last_24h_cents as f64);
    v.put(Feature::MerchantRefundRate, order.merchant_refund_rate);
    v.flag(Feature::ShippingMismatch, order.shipping_mismatch);
    v.put(Feature::CustomerPiiFields, f64::from(order.customer_pii_fields));
    v.put(Feature::DiscountRatio, order.discount_ratio);
    v.flag(Feature::DigitalGoods, order.digital_goods);
    v.flag(Feature::ManualOverrideRequested, order.manual_override_requested);
    v.flag(Feature::UnformattedSensitiveFields, order.unformatted_sensitive_fields);
    if let Some(exec) = exec {
        exec.apply(&mut v);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_storage::FileChange;
    use std::path::PathBuf;

    fn change(path: &str, kind: ChangeKind, added: &str) -> FileChange {
        FileChange {
            path: PathBuf::from(path),
            kind,
            old_len: 10,
            new_len: 10 + added.len() as u64,
            is_symlink: false,
            mode_changed: false,
            binary: false,
            lines_added: added.lines().count() as u32,
            lines_removed: 0,
            added_text: added.to_string(),
            removed_text: String::new(),
        }
    }

    #[test]
    fn rust_table_matches_feature_schema_json() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../models/feature_schema.json")).expect("schema JSON");
        let feats = schema["features"].as_array().expect("features array");
        assert_eq!(feats.len(), FEATURE_COUNT);
        for (i, f) in feats.iter().enumerate() {
            assert_eq!(f["index"].as_u64(), Some(i as u64));
            assert_eq!(f["name"].as_str(), Some(FEATURE_NAMES[i]), "name of feature {i}");
            let expected = match (f["kind"].as_str().unwrap(), f["cap"].as_f64()) {
                ("binary", _) => Kind::Binary,
                ("ratio", _) => Kind::Ratio,
                ("count", Some(cap)) => Kind::Count(cap),
                ("log", Some(cap)) => Kind::Log(cap),
                other => panic!("unexpected kind {other:?}"),
            };
            assert_eq!(FEATURE_KINDS[i], expected, "kind of {}", FEATURE_NAMES[i]);
        }
    }

    #[test]
    fn normalisation_follows_kind() {
        assert_eq!(Kind::Count(64.0).normalize(32.0), 0.5);
        assert_eq!(Kind::Count(4.0).normalize(9.0), 1.0);
        assert!((Kind::Log(1000.0).normalize(1000.0) - 1.0).abs() < 1e-6);
        assert_eq!(Kind::Binary.normalize(3.0), 1.0);
        assert_eq!(Kind::Ratio.normalize(f64::NAN), 0.0);
    }

    #[test]
    fn env_secret_change_lights_up_risk_features() {
        let key = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        let changes = ChangeSet {
            files: vec![change(".env", ChangeKind::Modified, &format!("AWS_ACCESS_KEY_ID={key}"))],
            scanned_entries: 2,
        };
        let v = workspace_features(&changes, &ExecutionContext::default());
        assert_eq!(v.get(Feature::EnvFileTouched), 1.0);
        assert_eq!(v.get(Feature::SecretPatternHits), 0.25);
        assert_eq!(v.get(Feature::UnformattedSensitiveFields), 1.0);
    }

    #[test]
    fn test_and_doc_ratios() {
        let changes = ChangeSet {
            files: vec![
                change("src/lib.rs", ChangeKind::Modified, "pub fn a() {}"),
                change("tests/a_test.rs", ChangeKind::Added, "#[test] fn t() {}"),
                change("README.md", ChangeKind::Modified, "docs"),
                change("policies/aegis_rules.cedar", ChangeKind::Modified, "permit(principal, action, resource);"),
            ],
            scanned_entries: 8,
        };
        let v = workspace_features(&changes, &ExecutionContext { retry_count: 2, ..Default::default() });
        assert_eq!(v.get(Feature::TestFilesRatio), 0.25);
        assert_eq!(v.get(Feature::DocsFilesRatio), 0.25);
        assert_eq!(v.get(Feature::PolicyFileTouched), 1.0);
        assert_eq!(v.get(Feature::RetryCount), 0.4);
        assert_eq!(v.get(Feature::FilesTouched), 4.0 / 64.0);
    }

    #[test]
    fn commerce_features_cover_budget_and_receipt() {
        let order = CommerceContext {
            amount_cents: 12_000,
            budget_cents: 20_000,
            currency: "USD".into(),
            receipt_verified: true,
            line_items: 2,
            ..Default::default()
        };
        let v = commerce_features(&order, None);
        assert!((v.get(Feature::BudgetUtilization) - 0.6).abs() < 1e-6);
        assert_eq!(v.get(Feature::ReceiptUnverified), 0.0);
        assert_eq!(v.get(Feature::CurrencyNonUsd), 0.0);
        assert_eq!(v.get(Feature::FilesTouched), 0.0);
    }
}

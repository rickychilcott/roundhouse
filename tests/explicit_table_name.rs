//! `self.table_name = "remote_domains"` binds a model to that table.
//!
//! Shopify's `DomainSubscription` stores in `remote_domains`. Ingest
//! derived the conventional `domain_subscriptions`, found no such table
//! and gave the model NO columns, so `subscription.shop_id` (a column
//! of `remote_domains`) read as "no known method `shop_id` on
//! DomainSubscription?" -- 38 errors on core, and 783 models declare
//! their table this way.

use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::analyze::{diagnose, Analyzer};
use roundhouse::ingest::ingest_app_from_tree;

fn failures(model_body: &str) -> Vec<String> {
    let files: Vec<(&str, String)> = vec![
        (
            "db/schema.rb",
            "ActiveRecord::Schema.define do\n  create_table \"remote_domains\", force: :cascade do |t|\n    t.integer \"shop_id\"\n  end\nend\n".into(),
        ),
        (
            "app/models/domain_subscription.rb",
            format!("class DomainSubscription < ApplicationRecord\n{model_body}\n  def self.owner\n    find_by(id: 1).shop_id\n  end\nend\n"),
        ),
    ];
    let tree: HashMap<PathBuf, Vec<u8>> =
        files.into_iter().map(|(p, c)| (PathBuf::from(p), c.into_bytes())).collect();
    let mut app = ingest_app_from_tree(tree).expect("ingest");
    Analyzer::new(&app).analyze(&mut app);
    diagnose(&app)
        .iter()
        .filter(|d| d.code() == "send_dispatch_failed")
        .map(|d| format!("{d:?}"))
        .collect()
}

#[test]
fn a_renamed_table_supplies_the_models_columns() {
    let f = failures("  self.table_name = \"remote_domains\"");
    assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
}

#[test]
fn without_the_declaration_the_conventional_table_is_used() {
    let f = failures("");
    assert_eq!(f.len(), 1, "no domain_subscriptions table, so no shop_id: {f:?}");
}

/// `self.table_name_prefix = "three_d_secure_"` in the class body (32
/// core models, e.g. `ThreeDSecure::Authentication`) prefixes the
/// derived name the way a namespace module's `table_name_prefix` does.
#[test]
fn a_class_level_table_name_prefix_is_applied() {
    let files: Vec<(&str, String)> = vec![
        (
            "db/schema.rb",
            "ActiveRecord::Schema.define do\n  create_table \"three_d_secure_authentications\", force: :cascade do |t|\n    t.string \"status\"\n  end\nend\n".into(),
        ),
        (
            "app/models/three_d_secure/authentication.rb",
            "module ThreeDSecure\n  class Authentication < ApplicationRecord\n    self.table_name_prefix = \"three_d_secure_\"\n\n    def self.st\n      find_by(id: 1).status\n    end\n  end\nend\n".into(),
        ),
    ];
    let tree: HashMap<PathBuf, Vec<u8>> =
        files.into_iter().map(|(p, c)| (PathBuf::from(p), c.into_bytes())).collect();
    let mut app = ingest_app_from_tree(tree).expect("ingest");
    Analyzer::new(&app).analyze(&mut app);
    let f: Vec<_> = diagnose(&app)
        .iter()
        .filter(|d| d.code() == "send_dispatch_failed")
        .map(|d| format!("{d:?}"))
        .collect();
    assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
}

/// Rails' `compute_table_name`: a class nested under a concrete model is
/// stored in the parent's singular table name plus its own plural, so
/// `Community::Advocate` reads `community_advocates`. Every case below
/// declares a decoy table the wrong derivation would pick, and the probe
/// reads a column only the right table has.
mod nested_under_a_model {
    use super::*;

    fn schema(tables: &[(&str, &str)]) -> String {
        let mut out = String::from("ActiveRecord::Schema.define do\n");
        for (table, column) in tables {
            out += &format!(
                "  create_table \"{table}\", force: :cascade do |t|\n    t.string \"{column}\"\n  end\n"
            );
        }
        out + "end\n"
    }

    fn probe(class_line: &str, column: &str) -> String {
        format!("{class_line}\n  def self.probe\n    find_by(id: 1).{column}\n  end\nend\n")
    }

    fn dispatch_failures(files: Vec<(&str, String)>) -> Vec<String> {
        let tree: HashMap<PathBuf, Vec<u8>> = files
            .into_iter()
            .map(|(p, c)| (PathBuf::from(p), c.into_bytes()))
            .collect();
        let mut app = ingest_app_from_tree(tree).expect("ingest");
        Analyzer::new(&app).analyze(&mut app);
        diagnose(&app)
            .iter()
            .filter(|d| d.code() == "send_dispatch_failed")
            .map(|d| format!("{d:?}"))
            .collect()
    }

    #[test]
    fn a_class_under_a_model_reads_the_parents_singular_table() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[
                    ("communities", "name"),
                    ("advocates", "decoy"),
                    ("community_advocates", "first_name"),
                    ("profiles", "decoy"),
                    ("community_advocate_profiles", "bio"),
                ]),
            ),
            (
                "app/models/community.rb",
                "class Community < ApplicationRecord\nend\n".into(),
            ),
            (
                "app/models/community/advocate.rb",
                probe(
                    "class Community::Advocate < ApplicationRecord",
                    "first_name",
                ),
            ),
            (
                "app/models/community/advocate/profile.rb",
                probe(
                    "class Community::Advocate::Profile < ApplicationRecord",
                    "bio",
                ),
            ),
        ]);
        assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
    }

    #[test]
    fn the_same_shape_under_a_plain_module_stays_demodulized() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[("devices", "token"), ("push_devices", "decoy")]),
            ),
            ("app/models/push.rb", "module Push\nend\n".into()),
            (
                "app/models/push/device.rb",
                probe("class Push::Device < ApplicationRecord", "token"),
            ),
        ]);
        assert!(
            f.is_empty(),
            "a plain module is not a table-name parent: {f:?}"
        );
    }

    #[test]
    fn an_abstract_parent_does_not_join_the_table_name() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[("widgets", "sku"), ("tenant_widgets", "decoy")]),
            ),
            (
                "app/models/tenant.rb",
                "class Tenant < ApplicationRecord\n  self.abstract_class = true\nend\n".into(),
            ),
            (
                "app/models/tenant/widget.rb",
                probe("class Tenant::Widget < ApplicationRecord", "sku"),
            ),
        ]);
        assert!(
            f.is_empty(),
            "an abstract class is not a table-name parent: {f:?}"
        );
    }

    /// `Premium` is a single-table-inheritance child of `Community`, so
    /// its table is `communities` and its children are
    /// `community_perks` -- not `premium_perks`, and not
    /// `community_premia_perks`.
    #[test]
    fn a_single_table_inheritance_parent_contributes_its_base_table() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[
                    ("communities", "name"),
                    ("community_perks", "label"),
                    ("premium_perks", "decoy"),
                ]),
            ),
            (
                "app/models/community.rb",
                "class Community < ApplicationRecord\nend\n".into(),
            ),
            (
                "app/models/community/premium.rb",
                "class Community::Premium < Community\nend\n".into(),
            ),
            (
                "app/models/community/premium/perk.rb",
                probe(
                    "class Community::Premium::Perk < ApplicationRecord",
                    "label",
                ),
            ),
        ]);
        assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
    }

    #[test]
    fn a_declared_table_on_the_parent_is_the_one_singularized() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[
                    ("tenants", "name"),
                    ("tenant_advocates", "first_name"),
                    ("community_advocates", "decoy"),
                ]),
            ),
            (
                "app/models/community.rb",
                "class Community < ApplicationRecord\n  self.table_name = \"tenants\"\nend\n"
                    .into(),
            ),
            (
                "app/models/community/advocate.rb",
                probe(
                    "class Community::Advocate < ApplicationRecord",
                    "first_name",
                ),
            ),
        ]);
        assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
    }

    #[test]
    fn a_declared_table_on_the_nested_class_still_wins() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[("communities", "name"), ("roster", "first_name"), ("community_advocates", "decoy")]),
            ),
            ("app/models/community.rb", "class Community < ApplicationRecord\nend\n".into()),
            (
                "app/models/community/advocate.rb",
                "class Community::Advocate < ApplicationRecord\n  self.table_name = \"roster\"\n  def self.probe\n    find_by(id: 1).first_name\n  end\nend\n".into(),
            ),
        ]);
        assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
    }

    /// The parent's own name carries its namespace prefix; the module's
    /// prefix is not added a second time for the child, because the
    /// parent model is the first module parent that answers
    /// `table_name_prefix`.
    #[test]
    fn a_namespace_prefix_is_applied_once() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[
                    ("legacy_communities", "name"),
                    ("legacy_community_advocates", "first_name"),
                    ("legacy_legacy_community_advocates", "decoy"),
                ]),
            ),
            (
                "app/models/legacy.rb",
                "module Legacy\n  def self.table_name_prefix\n    \"legacy_\"\n  end\nend\n".into(),
            ),
            (
                "app/models/legacy/community.rb",
                "class Legacy::Community < ApplicationRecord\nend\n".into(),
            ),
            (
                "app/models/legacy/community/advocate.rb",
                probe(
                    "class Legacy::Community::Advocate < ApplicationRecord",
                    "first_name",
                ),
            ),
        ]);
        assert!(f.is_empty(), "unexpected dispatch failures: {f:?}");
    }

    /// Control: with `Community` a plain module the same files must NOT
    /// find the nested table, so the cases above are not vacuous.
    #[test]
    fn without_a_model_parent_the_nested_table_is_not_found() {
        let f = dispatch_failures(vec![
            (
                "db/schema.rb",
                schema(&[
                    ("advocates", "decoy"),
                    ("community_advocates", "first_name"),
                ]),
            ),
            ("app/models/community.rb", "module Community\nend\n".into()),
            (
                "app/models/community/advocate.rb",
                probe(
                    "class Community::Advocate < ApplicationRecord",
                    "first_name",
                ),
            ),
        ]);
        assert_eq!(
            f.len(),
            1,
            "reads advocates, which has no first_name: {f:?}"
        );
    }
}

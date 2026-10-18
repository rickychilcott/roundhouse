//! Dropped model declarations belong on the ingest-gap ledger without
//! changing analysis or emitted behavior. AASM and state_machine are
//! the currently recognized declaration fixtures, not runtime support
//! or a claim that every unknown class-body statement is reported.

use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::analyze::{Analyzer, attribution::attribute_ingest_gaps, diagnose};
use roundhouse::diagnostic::{DiagnosticKind, Severity};
use roundhouse::dialect::{Association, Dependent, ModelBodyItem};
use roundhouse::expr::ExprNode;
use roundhouse::ingest::{IngestError, ingest_app_from_tree, survey};

fn tree(model: &str, concern: &str) -> HashMap<PathBuf, Vec<u8>> {
    [
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\nend\n"),
        ("app/models/article.rb", model),
        ("app/models/ticket.rb", "class Ticket < ApplicationRecord\n  include Workflow\nend\n"),
        ("app/models/concerns/workflow.rb", concern),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/articles_controller.rb", "class ArticlesController < ApplicationController\n  def show\n    @article = Article.find(params[:id])\n    @article.publish! if @article.may_publish?\n    render plain: 'ok'\n  end\nend\n"),
        ("config/routes.rb", "Rails.application.routes.draw do\n  resources :articles, only: :show\nend\n"),
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table :articles do |t|\n    t.string :title\n  end\n  create_table :tickets do |t|\n    t.string :state\n  end\nend\n"),
    ].into_iter().map(|(p, s)| (PathBuf::from(p), s.as_bytes().to_vec())).collect()
}

fn entries(gaps: &[IngestError]) -> Vec<(&str, &str)> {
    gaps.iter()
        .map(|g| match g {
            IngestError::Unsupported { file, message } => (file.as_str(), message.as_str()),
            other => panic!("unexpected gap: {other}"),
        })
        .collect()
}

#[test]
fn dropped_declarations_are_reported_once_without_changing_analysis_or_emission() {
    let files = tree(
        r#"class Article < ApplicationRecord
  include Workflow
  aasm column: :state do
    state :draft, initial: true
    state :published
    event :publish do
      transitions from: :draft, to: :published
    end
  end
  aasm :review do
    state :pending, initial: true
  end
  state_machine :status, initial: :open do
    event :close do
      transition open: :closed
    end
  end
  def headline
    title
  end
end
"#,
        r#"module Workflow
  extend ActiveSupport::Concern
  included do
    aasm :delivery do
      state :waiting, initial: true
      state :delivered
      event :deliver do
        transitions from: :waiting, to: :delivered
      end
    end
    with_options do
      state_machine :delivery_status, initial: :waiting do
        event :deliver do
          transition waiting: :delivered
        end
      end
    end
  end
end
"#,
    );
    // Deliberately no Gemfile.lock: these entries name unmodeled DSL
    // shapes, not proven ownership by a particular installed gem.
    let mut strict = ingest_app_from_tree(files.clone()).expect("strict ingest");
    survey::activate();
    let result = ingest_app_from_tree(files);
    let gaps = survey::drain();
    let mut surveyed = result.expect("survey ingest");
    let mut actual = entries(&gaps);
    actual.sort();
    assert_eq!(
        actual,
        vec![
            (
                "app/models/article.rb",
                "state-machine DSL block `aasm` is not modeled"
            ),
            (
                "app/models/article.rb",
                "state-machine DSL block `aasm` is not modeled"
            ),
            (
                "app/models/article.rb",
                "state-machine DSL block `state_machine` is not modeled"
            ),
            (
                "app/models/concerns/workflow.rb",
                "state-machine DSL block `aasm` is not modeled"
            ),
            (
                "app/models/concerns/workflow.rb",
                "state-machine DSL block `state_machine` is not modeled"
            ),
        ]
    );
    assert_eq!(surveyed, strict, "the ledger must not change ingested IR");
    let declarations = |body: &[ModelBodyItem]| {
        body.iter()
            .filter(|item| {
                matches!(item, ModelBodyItem::Unknown { expr, .. }
            if matches!(&*expr.node, ExprNode::Send { method, block: Some(_), .. }
                if matches!(method.as_str(), "aasm" | "state_machine")))
            })
            .count()
    };
    let article = surveyed
        .models
        .iter()
        .find(|m| m.name.0.as_str() == "Article")
        .unwrap();
    assert_eq!(
        declarations(&article.body),
        3,
        "model declarations still stay Unknown"
    );
    let ticket = surveyed
        .models
        .iter()
        .find(|m| m.name.0.as_str() == "Ticket")
        .unwrap();
    assert_eq!(
        declarations(&ticket.body),
        0,
        "unsupported concern items are not spliced"
    );
    Analyzer::new(&strict).analyze(&mut strict);
    Analyzer::new(&surveyed).analyze(&mut surveyed);
    assert_eq!(surveyed, strict, "no typed methods are added");
    let raw = diagnose(&surveyed);
    assert_eq!(raw, diagnose(&strict));
    let unresolved = raw
        .iter()
        .position(|d| matches!(&d.kind, DiagnosticKind::SendDispatchFailed { method, .. } if method.as_str() == "may_publish?"))
        .expect("the unmodeled call must remain unresolved");
    assert_eq!(raw[unresolved].severity, Severity::Error);
    // Existing file/class gap attribution does affect presentation.
    // This is a coverage note, not a newly supported generated method.
    let mut attributed = raw.clone();
    attribute_ingest_gaps(&mut attributed, &surveyed, &gaps);
    assert_eq!(attributed[unresolved].severity, Severity::Info);
    assert_eq!(attributed[unresolved].kind, raw[unresolved].kind);
    assert!(
        attributed[unresolved]
            .message
            .contains("state-machine DSL block")
    );
    roundhouse::session::analyze_and_lower(&mut strict);
    roundhouse::session::analyze_and_lower(&mut surveyed);
    assert_eq!(
        roundhouse::emit::ruby::emit_lowered_models(&surveyed),
        roundhouse::emit::ruby::emit_lowered_models(&strict)
    );
}

#[test]
fn ordinary_calls_do_not_create_declaration_gaps() {
    survey::activate();
    let result = ingest_app_from_tree(tree(
        r#"class Article < ApplicationRecord
  aasm
  state_machine :state
  aasm(&-> {})
  state_machine(&-> {})
  self.aasm do; end
  Registry.state_machine do; end
  another_dsl do; end
  def ordinary
    aasm do; end
    state_machine do; end
  end
end
"#,
        "module Workflow\n  extend ActiveSupport::Concern\nend\n",
    ));
    let gaps = survey::drain();
    result.expect("non-declaration controls ingest");
    assert!(gaps.is_empty(), "unexpected gaps: {gaps:?}");
}

#[test]
fn declaration_reporting_preserves_original_ingest_failures() {
    let files = tree(
        "class Article < ApplicationRecord\n  aasm do\n    defined?(helper(1))\n  end\nend\n",
        "module Workflow\nend\n",
    );
    let strict_error = ingest_app_from_tree(files.clone()).expect_err("strict ingest still fails");
    survey::activate();
    let result = ingest_app_from_tree(files);
    let gaps = survey::drain();
    result.expect("survey recovers");
    let actual = entries(&gaps);
    assert_eq!(
        actual
            .iter()
            .filter(|entry| **entry
                == (
                    "app/models/article.rb",
                    "state-machine DSL block `aasm` is not modeled"
                ))
            .count(),
        1,
        "one declaration entry even when its body fails"
    );
    let declaration = actual
        .iter()
        .position(|(_, message)| *message == "state-machine DSL block `aasm` is not modeled")
        .unwrap();
    assert!(
        actual
            .iter()
            .skip(declaration + 1)
            .any(|(_, message)| strict_error.to_string().contains(message)),
        "original body failure recorded after the declaration: {strict_error}"
    );
}

fn model<'a>(app: &'a roundhouse::App, name: &str) -> &'a roundhouse::dialect::Model {
    app.models
        .iter()
        .find(|model| model.name.0.as_str() == name)
        .expect(name)
}

fn association<'a>(model: &'a roundhouse::dialect::Model, name: &str) -> Option<&'a Association> {
    model
        .associations()
        .find(|assoc| assoc.name().as_str() == name)
}

/// `with_options` merges its hash into each association inside, the call's
/// own keys winning, and nested blocks compose, in a model body and in a
/// concern's `included do` alike.
#[test]
fn with_options_merges_into_each_association_and_the_calls_keys_win() {
    let body = r#"
  with_options dependent: :destroy, class_name: "Remark" do
    has_many :comments, class_name: "Note"
    has_many :replies, dependent: :nullify
    with_options foreign_key: :post_id do
      has_one :cover
      belongs_to :section
    end
  end
"#;
    let app = ingest_app_from_tree(tree(
        &format!("class Article < ApplicationRecord{body}end\n"),
        &format!(
            "module Workflow\n  extend ActiveSupport::Concern\n  included do{body}  end\nend\n"
        ),
    ))
    .expect("ingest");
    for owner in ["Article", "Ticket"] {
        let model = model(&app, owner);
        let Some(Association::HasMany {
            target, dependent, ..
        }) = association(model, "comments")
        else {
            panic!("{owner}: comments is not a has_many");
        };
        assert_eq!(
            (target.0.as_str(), dependent),
            ("Note", &Dependent::Destroy),
            "{owner}: own class_name wins"
        );
        let Some(Association::HasMany {
            target, dependent, ..
        }) = association(model, "replies")
        else {
            panic!("{owner}: replies is not a has_many");
        };
        assert_eq!(
            (target.0.as_str(), dependent),
            ("Remark", &Dependent::Nullify),
            "{owner}: own dependent wins"
        );
        let Some(Association::HasOne {
            target,
            foreign_key,
            dependent,
            ..
        }) = association(model, "cover")
        else {
            panic!("{owner}: cover is not a has_one");
        };
        assert_eq!(
            (target.0.as_str(), foreign_key.as_str(), dependent),
            ("Remark", "post_id", &Dependent::Destroy),
            "{owner}: nested blocks compose"
        );
        let Some(Association::BelongsTo {
            target,
            foreign_key,
            ..
        }) = association(model, "section")
        else {
            panic!("{owner}: section is not a belongs_to");
        };
        assert_eq!(
            (target.0.as_str(), foreign_key.as_str()),
            ("Remark", "post_id"),
            "{owner}"
        );
    }
}

/// A `with_options` block that holds anything but association macros is
/// not composed: it is recorded as a gap and registers nothing, rather than
/// registering the associations without the options or the validation
/// without its `if:`.
#[test]
fn with_options_around_other_declarations_is_a_gap_and_registers_nothing() {
    let body = r#"
  with_options if: :published? do
    has_many :comments
    validates :title, presence: true
  end
"#;
    let files = tree(
        &format!("class Article < ApplicationRecord{body}end\n"),
        &format!(
            "module Workflow\n  extend ActiveSupport::Concern\n  included do{body}  end\nend\n"
        ),
    );
    ingest_app_from_tree(files.clone()).expect("strict ingest declines without failing");
    survey::activate();
    let result = ingest_app_from_tree(files);
    let gaps = survey::drain();
    let app = result.expect("survey ingest");
    let actual = entries(&gaps);
    let want =
        "with_options: `validates` is not an association macro; only associations are composed";
    for file in ["app/models/article.rb", "app/models/concerns/workflow.rb"] {
        assert_eq!(
            actual
                .iter()
                .filter(|entry| **entry == (file, want))
                .count(),
            1,
            "{file}: {actual:?}"
        );
    }
    for owner in ["Article", "Ticket"] {
        let model = model(&app, owner);
        assert!(
            association(model, "comments").is_none(),
            "{owner}: the association must not register"
        );
        assert!(
            model.validations().next().is_none(),
            "{owner}: the validation must not register unconditionally"
        );
    }
}

//! A concern's class-side methods reach the model when spelled as
//! `class << self` directly inside `included do`, the fourth spelling
//! beside `class_methods do`, `module ClassMethods` and the vanilla
//! `def self.included(klass); class << klass` hook
//! (`tests/concern_included_hook_class_methods.rs`).
//!
//! ```ruby
//! module Labelled
//!   extend ActiveSupport::Concern
//!
//!   included do
//!     class << self
//!       def label_for(action) = "#{model_name.human} #{action}"
//!     end
//!   end
//! end
//! ```
//!
//! ActiveSupport::Concern runs the block with `class_eval` on the includer,
//! so `self` there is the includer and the singleton is the includer's own:
//! `Article.label_for`. Before the fix those defs reached no includer, the
//! module emitted empty, and no survey gap said so. The runtime half is
//! pinned in `tests/emit_and_run.rs` (`included_do_class_self_*`).

use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::analyze::{Analyzer, diagnose};
use roundhouse::diagnostic::Severity;
use roundhouse::emit::ruby;
use roundhouse::ingest::{ingest_app_from_tree, survey};

fn tree(files: &[(&str, &str)]) -> HashMap<PathBuf, Vec<u8>> {
    files
        .iter()
        .map(|(p, c)| (PathBuf::from(*p), c.as_bytes().to_vec()))
        .collect()
}

const SCHEMA: &str = r#"ActiveRecord::Schema.define do
  create_table "articles", force: :cascade do |t|
    t.string "title", null: false
  end
end
"#;

fn labelled(included_body: &str) -> String {
    format!(
        "module Labelled\n  extend ActiveSupport::Concern\n\n  included do\n{included_body}  end\nend\n"
    )
}

const SINGLETON: &str =
    "    class << self\n      def label_for(action) = \"article #{action}\"\n    end\n";

fn files<'a>(article: &'a str, concern: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("db/schema.rb", SCHEMA),
        ("app/models/article.rb", article),
        ("app/models/concerns/labelled.rb", concern),
        (
            "app/services/consumer.rb",
            "class Consumer\n  def self.call\n    Article.label_for(:create)\n  end\nend\n",
        ),
    ]
}

const PLAIN_ARTICLE: &str = "class Article < ApplicationRecord\n  include Labelled\nend\n";

fn article_rb(app: &roundhouse::App) -> String {
    ruby::emit_lowered_models(app)
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("app/models/article.rb"))
        .map(|f| f.content.clone())
        .expect("article.rb")
}

fn errors(app: &mut roundhouse::App) -> Vec<String> {
    Analyzer::new(app).analyze(app);
    diagnose(app)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{:?}", d.kind))
        .collect()
}

/// Ingest in survey mode and hand back the app with every recorded gap.
fn surveyed(input: &[(&str, &str)]) -> (Result<roundhouse::App, String>, Vec<String>) {
    survey::activate();
    let result = ingest_app_from_tree(tree(input)).map_err(|e| e.to_string());
    let gaps = survey::drain().iter().map(|e| e.to_string()).collect();
    (result, gaps)
}

#[test]
fn class_self_in_included_do_reaches_the_including_model() {
    let mut app =
        ingest_app_from_tree(tree(&files(PLAIN_ARTICLE, &labelled(SINGLETON)))).expect("ingest");
    let article = article_rb(&app);
    assert!(
        article.contains("def self.label_for(action)"),
        "the class method lands on the model:\n{article}"
    );
    assert_eq!(
        errors(&mut app),
        Vec::<String>::new(),
        "Article.label_for resolves"
    );
}

/// Every def in the singleton block lands, and a def beside it in
/// `included do` is a different (instance) thing that is not turned into
/// a class method.
#[test]
fn every_def_in_the_block_lands_and_instance_defs_stay_instance() {
    let body = "    class << self\n      def label_for(action) = \"article #{action}\"\n      def other_for(action) = label_for(action).upcase\n    end\n\n    def label = \"instance\"\n";
    let app = ingest_app_from_tree(tree(&files(PLAIN_ARTICLE, &labelled(body)))).expect("ingest");
    let article = article_rb(&app);
    assert!(article.contains("def self.label_for(action)"), "{article}");
    assert!(article.contains("def self.other_for(action)"), "{article}");
    assert!(
        !article.contains("def self.label\n"),
        "the instance def is not a class method:\n{article}"
    );
}

/// The claim must survive the DSL the same block usually carries.
#[test]
fn the_method_still_resolves_beside_class_attribute_and_cattr_accessor() {
    let body = format!(
        "    class_attribute :label_prefix, default: \"article\"\n    cattr_accessor :label_count, default: 0\n\n{SINGLETON}"
    );
    let mut app =
        ingest_app_from_tree(tree(&files(PLAIN_ARTICLE, &labelled(&body)))).expect("ingest");
    let article = article_rb(&app);
    assert!(article.contains("def self.label_for(action)"), "{article}");
    assert_eq!(errors(&mut app), Vec::<String>::new());
}

/// A `class << self` that is not a direct statement of the block is not
/// claimed, and says so rather than vanishing. Under an `if` the
/// visibility gate already refuses the whole file; under `with_options`
/// nothing did, and the defs were dropped silently.
#[test]
fn a_nested_class_self_records_a_gap() {
    let conditional = format!(
        "    if respond_to?(:abstract_class)\n  {}    end\n",
        SINGLETON.replace('\n', "\n  ")
    );
    let (result, gaps) = surveyed(&files(PLAIN_ARTICLE, &labelled(&conditional)));
    result.expect("survey keeps going");
    assert!(
        gaps.iter().any(|g| g.contains("labelled.rb")),
        "conditional: {gaps:?}"
    );

    let wrapped = format!("    with_options if: :always do\n{SINGLETON}    end\n");
    let (result, gaps) = surveyed(&files(PLAIN_ARTICLE, &labelled(&wrapped)));
    result.expect("survey keeps going");
    assert!(
        gaps.iter()
            .any(|g| g.contains("`class << self` nested inside an `included do` block")),
        "with_options: {gaps:?}"
    );
}

/// Taking a `class << self` out of `included do` takes its defs out of the
/// block that is kept, so they are not also refused as nested defs. A def
/// that was NOT claimed (here the same shape under `with_options`) still is.
#[test]
fn claimed_defs_are_not_nested_defs_but_unclaimed_ones_still_are() {
    fn nested_def_errors(included_body: &str) -> usize {
        let (result, _) = surveyed(&files(PLAIN_ARTICLE, &labelled(included_body)));
        let mut app = result.expect("ingest");
        Analyzer::new(&app).analyze(&mut app);
        diagnose(&app)
            .iter()
            .filter(|d| {
                d.severity == Severity::Error && format!("{:?}", d.kind).contains("nested def")
            })
            .count()
    }
    assert_eq!(nested_def_errors(SINGLETON), 0);
    let wrapped = format!("    with_options if: :always do\n{SINGLETON}    end\n");
    assert!(
        nested_def_errors(&wrapped) > 0,
        "an unclaimed def keeps the refusal"
    );
}

#[test]
fn a_direct_class_self_records_no_gap() {
    let (result, gaps) = surveyed(&files(PLAIN_ARTICLE, &labelled(SINGLETON)));
    result.expect("ingest");
    assert!(gaps.is_empty(), "{gaps:?}");
}

/// Rails runs the `included do` def at include time, so the includer's own
/// definition wins only when it comes after the `include`.
#[test]
fn an_own_definition_after_the_include_wins_without_a_gap() {
    let article = "class Article < ApplicationRecord\n  include Labelled\n\n  def self.label_for(action) = \"own #{action}\"\nend\n";
    let (result, gaps) = surveyed(&files(article, &labelled(SINGLETON)));
    let app = result.expect("ingest");
    assert!(gaps.is_empty(), "{gaps:?}");
    let article = article_rb(&app);
    assert_eq!(
        article.matches("def self.label_for").count(),
        1,
        "{article}"
    );
    assert!(
        article.contains("own #"),
        "the model's own body is the one kept:\n{article}"
    );
}

/// An own definition BEFORE the `include` is replaced by the concern's in
/// Rails. The splice keeps the model's, so the collision is ledgered, and
/// strict mode refuses instead of emitting the wrong winner.
#[test]
fn an_own_definition_before_the_include_is_a_gap() {
    let article = "class Article < ApplicationRecord\n  def self.label_for(action) = \"own #{action}\"\n\n  include Labelled\nend\n";
    let (result, gaps) = surveyed(&files(article, &labelled(SINGLETON)));
    result.expect("survey keeps going");
    assert!(
        gaps.iter().any(
            |g| g.contains("`Labelled` defines class method `label_for`")
                && g.contains("source order")
        ),
        "{gaps:?}"
    );
    let strict = ingest_app_from_tree(tree(&files(article, &labelled(SINGLETON))))
        .err()
        .expect("strict refuses");
    assert!(strict.to_string().contains("label_for"), "{strict}");
}

/// A singleton def outranks an extended `ClassMethods` in Rails whatever
/// the include order; the splice goes by include order, so when the
/// `class_methods` concern comes first the collision is ledgered.
#[test]
fn a_class_methods_concern_of_the_same_name_is_a_gap_only_when_it_would_win() {
    let carrier = "module Named\n  extend ActiveSupport::Concern\n\n  class_methods do\n    def label_for(action) = \"named #{action}\"\n  end\nend\n";
    let input = |article: &'static str| -> Vec<(&'static str, String)> {
        let mut v: Vec<(&'static str, String)> = files(article, "")
            .into_iter()
            .map(|(p, c)| (p, c.to_string()))
            .collect();
        v.iter_mut()
            .find(|(p, _)| *p == "app/models/concerns/labelled.rb")
            .unwrap()
            .1 = labelled(SINGLETON);
        v.push(("app/models/concerns/named.rb", carrier.to_string()));
        v
    };
    let run = |article: &'static str| {
        let owned = input(article);
        let borrowed: Vec<(&str, &str)> = owned.iter().map(|(p, c)| (*p, c.as_str())).collect();
        surveyed(&borrowed).1
    };
    // `included do` def first: it is what Rails answers, and what the splice keeps.
    let gaps = run("class Article < ApplicationRecord\n  include Labelled\n  include Named\nend\n");
    assert!(gaps.is_empty(), "{gaps:?}");
    // The carrier first would win the splice but loses in Rails.
    let gaps = run("class Article < ApplicationRecord\n  include Named\n  include Labelled\nend\n");
    assert!(
        gaps.iter()
            .any(|g| g.contains("`Labelled` defines class method `label_for`")),
        "{gaps:?}"
    );
}

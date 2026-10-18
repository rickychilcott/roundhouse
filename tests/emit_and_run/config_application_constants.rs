//! Constants and `def self.` helpers an app declares in the namespace
//! module that wraps `class Application < Rails::Application` in
//! `config/application.rb` (`module Stoked; SUPPORT_EMAIL = "..."; …`) are
//! read anywhere as `Stoked::SUPPORT_EMAIL`. Rails runs the file before any
//! app code, so they exist for the whole process; the emitted program has
//! to define them too, not just stop reporting them.

use super::emit_and_run;

fn app(module_body: &str) -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit("config/application.rb", "module RealBlog\n", &format!("module RealBlog\n{module_body}\n"))
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            "  validates :title, presence: true\n\n  def contact\n    [RealBlog::SUPPORT_EMAIL, RealBlog::MAX_ARTICLES, RealBlog.signature]\n  end\n",
        )
}

const MODULE_BODY: &str = r#"  SUPPORT_EMAIL = "help@example.com"
  MAX_ARTICLES = 10

  def self.signature
    "Sent by #{SUPPORT_EMAIL}"
  end
"#;

#[test]
fn constants_and_helpers_in_the_application_module_run_in_emitted_ruby() {
    app(MODULE_BODY)
        .run_ruby(
            r##"got = Article.new.contact
raise "Article#contact answered #{got.inspect}" unless got == ["help@example.com", 10, "Sent by help@example.com"]"##,
        )
        .assert_passes();
}

#[test]
fn an_application_module_constant_with_a_computed_value_stays_reported() {
    let (_, errors) = app("  SUPPORT_EMAIL = Mailbox.address\n  MAX_ARTICLES = 10\n")
        .emit(roundhouse::project::BuildTarget::Ruby);
    assert!(
        errors.iter().any(|e| e.contains("SUPPORT_EMAIL")),
        "a constant whose value cannot be evaluated must stay an error, got: {errors:#?}"
    );
}

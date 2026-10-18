//! `with_options OPTS do … end` around associations merges OPTS into each
//! one, in a model body and in a concern's `included do`.
//!
//! Before, a model body registered no association at all (`article.comments`
//! was `no known method`), and a concern registered it with OPTS dropped, so
//! `check` was clean and `dependent: :destroy` silently never cascaded.
//! These run the emitted program: the association exists and the cascade
//! really deletes the rows.

use super::emit_and_run;

const CASCADE: &str = r#"article = Article.create!(title: "One", body: "A sufficiently long body.")
Comment.create!(article_id: article.id, commenter: "Ann", body: "first comment")
raise "comments: #{article.comments.size}" unless article.comments.size == 1
article.destroy
raise "comments left behind: #{Comment.count}" unless Comment.count == 0
puts "with_options cascade passed"
"#;

fn assert_cascades(run: emit_and_run::Run) {
    run.assert_passes();
    assert!(
        run.stdout.contains("with_options cascade passed"),
        "{}",
        run.stdout
    );
}

#[test]
fn with_options_in_a_model_body_registers_the_association_and_its_options() {
    assert_cascades(
        emit_and_run::real_blog()
            .edit(
                "app/models/article.rb",
                "  has_many :comments, dependent: :destroy\n",
                "  with_options dependent: :destroy do\n    has_many :comments\n  end\n",
            )
            .run_ruby(CASCADE),
    );
}

#[test]
fn with_options_in_a_concerns_included_block_keeps_its_options() {
    assert_cascades(
        emit_and_run::real_blog()
            .write(
                "app/models/concerns/discussable.rb",
                "module Discussable\n  extend ActiveSupport::Concern\n\n  included do\n    with_options dependent: :destroy do\n      has_many :comments\n    end\n  end\nend\n",
            )
            .edit(
                "app/models/article.rb",
                "  has_many :comments, dependent: :destroy\n",
                "  include Discussable\n",
            )
            .run_ruby(CASCADE),
    );
}

/// The call's own `dependent:` beats the block's: `comments.article_id` is
/// NOT NULL, so a block-wins implementation would nullify and fail instead
/// of destroying the rows.
#[test]
fn a_calls_own_option_beats_the_with_options_block() {
    assert_cascades(
        emit_and_run::real_blog()
            .edit(
                "app/models/article.rb",
                "  has_many :comments, dependent: :destroy\n",
                "  with_options dependent: :nullify do\n    has_many :comments, dependent: :destroy\n  end\n",
            )
            .run_ruby(CASCADE),
    );
}

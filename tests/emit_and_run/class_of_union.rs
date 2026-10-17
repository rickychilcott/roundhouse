use super::emit_and_run;

/// `record.class.count` over a record that is an Article or a Comment
/// used to be a `no known method count on Class` error: `.class` of an
/// unknown or union receiver answered the generic `Class`. The seeded
/// tables hold one article and two comments, so an answer taken from
/// the first arm only, or from the wrong class, reports the wrong total.
/// The methods live on the model because that is where `check` reports
/// the error; a plain class's method bodies are not diagnosed.
const CLASS_METHODS: &str = "  validates :title, presence: true

  def self.table_total(record)
    record.class.count
  end

  def self.totals(article_id, comment_id)
    [table_total(Article.find(article_id)), table_total(Comment.find(comment_id))]
  end

  def self.picked_total(flag, article_id, comment_id)
    record = flag ? Article.find(article_id) : Comment.find(comment_id)
    record.class.count
  end
";

const ASSERTIONS: &str = r#"
Comment.delete_all
Article.delete_all
article = Article.create!(title: "Title", body: "A sufficiently long body.")
comment = Comment.create!(article_id: article.id, commenter: "Ann", body: "a comment")
Comment.create!(article_id: article.id, commenter: "Bob", body: "another comment")
totals = Article.totals(article.id, comment.id)
raise "totals: #{totals.inspect}" unless totals == [1, 2]
picked = [Article.picked_total(true, article.id, comment.id), Article.picked_total(false, article.id, comment.id)]
raise "picked: #{picked.inspect}" unless picked == [1, 2]
puts "class of union passed"
"#;

#[test]
fn class_of_a_model_union_answers_each_arms_class_in_emitted_ruby() {
    let run = emit_and_run::real_blog()
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            CLASS_METHODS,
        )
        .run_ruby(ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("class of union passed"));
}

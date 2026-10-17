//! A `rate_limit name:` Ruby cannot spell as a method name still throttles.
//!
//! Rails takes any string `name:`; it only joins the cache key. Roundhouse
//! derives the method it synthesizes from the name, and a hyphen made
//! `def rate_limit_ip-hourly-limit` fail to parse, so the limit was
//! dropped from the emitted program with only a survey gap to say so.
//! Two names that normalize alike share a controller here, so the ordinal
//! that keeps their methods apart is exercised at request time too.

use super::emit_and_run;

fn overlay() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit(
            "config/routes.rb",
            "  root \"articles#index\"\n",
            "  root \"articles#index\"\n  get \"/widgets\", to: \"widgets#index\"\n  get \"/widgets/:id\", to: \"widgets#show\"\n",
        )
        .write(
            "app/controllers/widgets_controller.rb",
            r#"class WidgetsController < ApplicationController
  rate_limit to: 1, within: 1.hour, only: :index, name: "ip-hourly-limit"
  rate_limit to: 2, within: 1.hour, only: :show, name: "ip_hourly_limit"

  def index
    head :ok
  end

  def show
    head :ok
  end
end
"#,
        )
}

const ASSERTIONS: &str = r#"
def get(path)
  status, = Main.run_rack(
    "REQUEST_METHOD" => "GET", "PATH_INFO" => path, "QUERY_STRING" => "",
    "HTTP_HOST" => "localhost", "REMOTE_ADDR" => "203.0.113.7",
    "rack.input" => StringIO.new("")
  )
  status
end

answers = [get("/widgets"), get("/widgets"), get("/widgets/1"), get("/widgets/1"), get("/widgets/1")]
raise "answered #{answers.inspect}, want [200, 429, 200, 200, 429]" unless answers == [200, 429, 200, 200, 429]
puts "hyphenated rate limit passed"
"#;

#[test]
fn a_hyphenated_rate_limit_name_throttles_at_request_time() {
    let run = overlay().run_ruby(ASSERTIONS);
    run.assert_passes();
    assert!(
        run.stdout.contains("hyphenated rate limit passed"),
        "{}",
        run.stdout
    );
}

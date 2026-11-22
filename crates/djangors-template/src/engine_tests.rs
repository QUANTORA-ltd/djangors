use super::*;
use crate::render;
use djangors_core::StatusCode;
use serde::Serialize;
use std::fs;
use tempfile::TempDir;

fn create_temp_template_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("djangors_test_")
        .tempdir()
        .unwrap()
}

#[test]
fn test_loader_precedence() {
    let dir1 = create_temp_template_dir();
    let dir2 = create_temp_template_dir();

    let file_name = "test_template.html";

    fs::write(dir1.path().join(file_name), "dir1 content").unwrap();
    fs::write(dir2.path().join(file_name), "dir2 content").unwrap();

    // With dir1 first, dir1 should win
    let engine1 =
        TemplateEngine::new(vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()]).unwrap();
    let rendered1 = engine1.render(file_name, ()).unwrap();
    assert_eq!(rendered1, "dir1 content");

    // With dir2 first, dir2 should win
    let engine2 =
        TemplateEngine::new(vec![dir2.path().to_path_buf(), dir1.path().to_path_buf()]).unwrap();
    let rendered2 = engine2.render(file_name, ()).unwrap();
    assert_eq!(rendered2, "dir2 content");
}

#[test]
fn test_template_inheritance() {
    let dir = create_temp_template_dir();

    let base_content = "<html><body>{% block content %}{% endblock %}</body></html>";
    let child_content = "{% extends 'base.html' %}{% block content %}Hello World{% endblock %}";

    fs::write(dir.path().join("base.html"), base_content).unwrap();
    fs::write(dir.path().join("child.html"), child_content).unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();
    let rendered = engine.render("child.html", ()).unwrap();
    assert_eq!(rendered, "<html><body>Hello World</body></html>");
}

#[test]
fn test_auto_escaping_is_real() {
    let dir = create_temp_template_dir();

    fs::write(dir.path().join("unsafe.html"), "{{ value }}").unwrap();
    fs::write(dir.path().join("unsafe.txt"), "{{ value }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        value: &'static str,
    }

    let ctx = Context {
        value: "<script>alert(1)</script>",
    };

    let rendered_html = engine.render("unsafe.html", &ctx).unwrap();
    assert_eq!(rendered_html, "&lt;script&gt;alert(1)&lt;&#x2f;script&gt;");

    let rendered_txt = engine.render("unsafe.txt", &ctx).unwrap();
    assert_eq!(rendered_txt, "<script>alert(1)</script>");
}

#[test]
fn test_filter_pluralize() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("plural.html"), "item{{ count|pluralize }}").unwrap();
    fs::write(
        dir.path().join("plural_custom.html"),
        "cherr{{ count|pluralize('y,ies') }}",
    )
    .unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        count: i32,
    }

    assert_eq!(
        engine.render("plural.html", &Context { count: 1 }).unwrap(),
        "item"
    );
    assert_eq!(
        engine.render("plural.html", &Context { count: 2 }).unwrap(),
        "items"
    );
    assert_eq!(
        engine.render("plural.html", &Context { count: 0 }).unwrap(),
        "items"
    );

    assert_eq!(
        engine
            .render("plural_custom.html", &Context { count: 1 })
            .unwrap(),
        "cherry"
    );
    assert_eq!(
        engine
            .render("plural_custom.html", &Context { count: 2 })
            .unwrap(),
        "cherries"
    );
}

#[test]
fn test_filter_floatformat() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("float.html"), "{{ val|floatformat }}").unwrap();
    fs::write(
        dir.path().join("float_arg.html"),
        "{{ val|floatformat(2) }}",
    )
    .unwrap();
    fs::write(
        dir.path().join("float_neg.html"),
        "{{ val|floatformat(-2) }}",
    )
    .unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        val: f64,
    }

    assert_eq!(
        engine
            .render("float.html", &Context { val: 34.232 })
            .unwrap(),
        "34.2"
    );
    assert_eq!(
        engine
            .render("float.html", &Context { val: 34.000 })
            .unwrap(),
        "34"
    );
    assert_eq!(
        engine
            .render("float.html", &Context { val: 34.260 })
            .unwrap(),
        "34.3"
    );

    assert_eq!(
        engine
            .render("float_arg.html", &Context { val: 34.232 })
            .unwrap(),
        "34.23"
    );
    assert_eq!(
        engine
            .render("float_arg.html", &Context { val: 34.000 })
            .unwrap(),
        "34.00"
    );

    assert_eq!(
        engine
            .render("float_neg.html", &Context { val: 34.232 })
            .unwrap(),
        "34.23"
    );
    assert_eq!(
        engine
            .render("float_neg.html", &Context { val: 34.000 })
            .unwrap(),
        "34"
    );
}

#[test]
fn test_filter_truncatewords() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("trunc.html"), "{{ val|truncatewords(3) }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        val: &'static str,
    }

    assert_eq!(
        engine
            .render(
                "trunc.html",
                &Context {
                    val: "one two three four five"
                }
            )
            .unwrap(),
        "one two three…"
    );
    assert_eq!(
        engine
            .render("trunc.html", &Context { val: "one two" })
            .unwrap(),
        "one two"
    );
}

#[test]
fn test_filter_default() {
    let dir = create_temp_template_dir();
    fs::write(
        dir.path().join("default.html"),
        "{{ val|default('nothing', true) }}",
    )
    .unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context1 {
        val: Option<&'static str>,
    }
    assert_eq!(
        engine
            .render("default.html", &Context1 { val: None })
            .unwrap(),
        "nothing"
    );
    assert_eq!(
        engine
            .render(
                "default.html",
                &Context1 {
                    val: Some("something")
                }
            )
            .unwrap(),
        "something"
    );
}

#[test]
fn test_filter_date() {
    let dir = create_temp_template_dir();
    fs::write(
        dir.path().join("date.html"),
        "{{ val|date('Y-m-d H:i:s') }}",
    )
    .unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        val: String,
    }

    let ctx = Context {
        val: "2026-07-17T18:02:05+00:00".to_string(),
    };
    assert_eq!(
        engine.render("date.html", &ctx).unwrap(),
        "2026-07-17 18:02:05"
    );
}

#[test]
fn test_render_shortcut() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("index.html"), "Hello {{ name }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        name: &'static str,
    }

    let resp = render(&engine, "index.html", &Context { name: "Alice" }).unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(resp.body().to_vec()).unwrap(),
        "Hello Alice"
    );
}

#[test]
fn test_from_embedded_auto_escaping() {
    let engine = TemplateEngine::from_embedded(&[
        ("hello.html", "Hello {{ name }}!"),
        ("hello.txt", "Hello {{ name }}!"),
    ])
    .unwrap();

    #[derive(Serialize)]
    struct Context {
        name: &'static str,
    }

    let ctx = Context {
        name: "<script>alert(1)</script>",
    };

    let rendered_html = engine.render("hello.html", &ctx).unwrap();
    assert_eq!(
        rendered_html,
        "Hello &lt;script&gt;alert(1)&lt;&#x2f;script&gt;!"
    );

    let rendered_txt = engine.render("hello.txt", &ctx).unwrap();
    assert_eq!(rendered_txt, "Hello <script>alert(1)</script>!");
}

#[test]
fn test_missing_template() {
    let engine = TemplateEngine::new(vec![]).unwrap();
    let res = engine.render("does_not_exist.html", ());
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(matches!(err, TemplateError::NotFound { .. }));
}

#[test]
fn test_filter_intcomma() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("comma.html"), "{{ val|intcomma }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context<T> {
        val: T,
    }

    assert_eq!(
        engine
            .render("comma.html", &Context { val: 1234567 })
            .unwrap(),
        "1,234,567"
    );
    assert_eq!(
        engine.render("comma.html", &Context { val: 456 }).unwrap(),
        "456"
    );
    assert_eq!(
        engine
            .render("comma.html", &Context { val: -1234567 })
            .unwrap(),
        "-1,234,567"
    );
    assert_eq!(
        engine
            .render("comma.html", &Context { val: "1234.56" })
            .unwrap(),
        "1,234.56"
    );
}

#[test]
fn test_filter_filesizeformat() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("size.html"), "{{ val|filesizeformat }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        val: u64,
    }

    assert_eq!(
        engine.render("size.html", &Context { val: 500 }).unwrap(),
        "500 bytes"
    );
    assert_eq!(
        engine.render("size.html", &Context { val: 1024 }).unwrap(),
        "1.0 KB"
    );
    assert_eq!(
        engine.render("size.html", &Context { val: 1536 }).unwrap(),
        "1.5 KB"
    );
    assert_eq!(
        engine
            .render("size.html", &Context { val: 1048576 })
            .unwrap(),
        "1.0 MB"
    );
}

#[test]
fn test_filter_naturaltime() {
    let dir = create_temp_template_dir();
    fs::write(dir.path().join("time.html"), "{{ val|naturaltime }}").unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    #[derive(Serialize)]
    struct Context {
        val: String,
    }

    let now = chrono::Utc::now();

    // 1. Very recent ("just now")
    let recent_val = now.to_rfc3339();
    assert_eq!(
        engine
            .render("time.html", &Context { val: recent_val })
            .unwrap(),
        "just now"
    );

    // 2. Past offset (e.g. 5 minutes ago)
    let past_val = (now - chrono::Duration::seconds(300)).to_rfc3339();
    assert_eq!(
        engine
            .render("time.html", &Context { val: past_val })
            .unwrap(),
        "5 minutes ago"
    );

    // 3. Future offset (e.g. 5 minutes in the future)
    let future_val = (now + chrono::Duration::seconds(300)).to_rfc3339();
    assert_eq!(
        engine
            .render("time.html", &Context { val: future_val })
            .unwrap(),
        "in 5 minutes"
    );
}

#[test]
fn test_function_static() {
    let engine = TemplateEngine::from_embedded(&[
        ("default.html", "{{ static('css/app.css') }}"),
        ("slash.html", "{{ static('/js/main.js') }}"),
    ])
    .unwrap();

    assert_eq!(
        engine.render("default.html", ()).unwrap(),
        "/static/css/app.css"
    );
    assert_eq!(
        engine.render("slash.html", ()).unwrap(),
        "/static/js/main.js"
    );

    let mut custom_engine =
        TemplateEngine::from_embedded(&[("custom.html", "{{ static('img/logo.png') }}")]).unwrap();
    custom_engine.set_static_url("/assets/");
    assert_eq!(
        custom_engine.render("custom.html", ()).unwrap(),
        "/assets/img/logo.png"
    );

    custom_engine.set_static_url("/media");
    assert_eq!(
        custom_engine.render("custom.html", ()).unwrap(),
        "/media/img/logo.png"
    );
}

#[test]
fn test_function_csrf_token() {
    let engine = TemplateEngine::from_embedded(&[("form.html", "{{ csrf_token() }}")]).unwrap();

    // Standard token rendering
    let ctx = minijinja::context! {
        _csrf_token => "secret_123",
    };
    let rendered = engine.render("form.html", &ctx).unwrap();
    assert_eq!(
        rendered,
        r#"<input type="hidden" name="csrfmiddlewaretoken" value="secret_123">"#
    );

    // Token containing " and < must be HTML-attribute-escaped and marked safe (not double escaped)
    let sneaky_ctx = minijinja::context! {
        _csrf_token => r#"abc"def<ghi"#,
    };
    let sneaky_rendered = engine.render("form.html", &sneaky_ctx).unwrap();
    assert_eq!(
        sneaky_rendered,
        r#"<input type="hidden" name="csrfmiddlewaretoken" value="abc&quot;def&lt;ghi">"#
    );

    // Missing csrf_token context variable must return an error naming missing context key
    let res = engine.render("form.html", ());
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("csrf_token"));
}

#[test]
fn test_function_url() {
    let mut engine =
        TemplateEngine::from_embedded(&[("link.html", "{{ url('poll-detail', id=1) }}")]).unwrap();

    // 1. Resolver unset -> template error
    let res = engine.render("link.html", ());
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("No URL resolver configured"));

    // 2. Resolver configured -> success
    engine.set_url_resolver(|name, params| {
        if name == "poll-detail" {
            let id = params.iter().find(|(k, _)| k == "id")?.1.clone();
            Some(format!("/polls/{id}"))
        } else {
            None
        }
    });

    let rendered = engine.render("link.html", ()).unwrap();
    assert_eq!(rendered, "/polls/1");

    // 3. Unknown route name -> template error
    let engine_bad =
        TemplateEngine::from_embedded(&[("bad.html", "{{ url('unknown-route') }}")]).unwrap();
    let mut engine_bad = engine_bad;
    engine_bad.set_url_resolver(|_, _| None);
    let res_bad = engine_bad.render("bad.html", ());
    assert!(res_bad.is_err());
}

#[test]
fn test_function_now() {
    let engine = TemplateEngine::from_embedded(&[("year.html", "{{ now('%Y') }}")]).unwrap();

    let rendered = engine.render("year.html", ()).unwrap();
    assert_eq!(rendered.len(), 4);
    assert!(rendered.parse::<u32>().is_ok());
}

#[test]
fn test_function_csp_nonce() {
    let engine = TemplateEngine::from_embedded(&[(
        "s.html",
        "<script nonce=\"{{ csp_nonce() }}\"></script>",
    )])
    .unwrap();
    let rendered = engine
        .render("s.html", minijinja::context! { _csp_nonce => "abc123" })
        .unwrap();
    assert_eq!(rendered, "<script nonce=\"abc123\"></script>");

    let empty = engine.render("s.html", ()).unwrap();
    assert_eq!(empty, "<script nonce=\"\"></script>");
}

#[test]
fn test_context_processors_and_render_with_request() {
    use crate::context_processors::{
        CsrfContextProcessor, ExtraContextProcessor, RequestContextProcessor, StaticUrlProcessor,
    };
    use bytes::Bytes;
    use djangors_core::Request;
    use hyper::{HeaderMap, Method, Uri};

    let mut engine = TemplateEngine::from_embedded(&[(
        "page.html",
        "static={{ STATIC_URL }}|path={{ request.path }}|method={{ request.method }}|htmx={{ request.is_htmx }}|csrf={{ csrf_token() }}|site={{ site_name }}|title={{ title }}",
    )])
    .unwrap();

    engine.add_context_processor(Arc::new(StaticUrlProcessor::new("/assets/")));
    engine.add_context_processor(Arc::new(CsrfContextProcessor::new()));
    engine.add_context_processor(Arc::new(RequestContextProcessor::new()));
    engine.add_context_processor(Arc::new(
        ExtraContextProcessor::default()
            .with("site_name", "Djangors")
            .with("title", "Default Title"),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        "sessionid=xyz; csrftoken=tok_abc".parse().unwrap(),
    );
    headers.insert("hx-request", "true".parse().unwrap());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/articles?page=2"),
        headers,
        Bytes::new(),
    );

    // Extra context overrides processor keys (`title`), while keeping processor keys (`site_name`).
    let html = engine
        .render_with_request(&req, "page.html", minijinja::context! { title => "Home" })
        .unwrap();
    assert!(html.contains("static=&#x2f;assets&#x2f;"), "html: {html}");
    assert!(html.contains("path=&#x2f;articles"), "html: {html}");
    assert!(html.contains("method=GET"), "html: {html}");
    assert!(html.contains("htmx=true"), "html: {html}");
    assert!(
        html.contains(r#"<input type="hidden" name="csrfmiddlewaretoken" value="tok_abc">"#),
        "html: {html}"
    );
    assert!(html.contains("site=Djangors"), "html: {html}");
    assert!(html.contains("title=Home"), "html: {html}");

    // Shortcut helper returns 200 HTML Response
    let resp = crate::render_with_request(&engine, &req, "page.html", ()).unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert!(body.contains("title=Default Title"));
}

#[test]
fn test_function_querystring() {
    let engine = TemplateEngine::from_embedded(&[
        (
            "qs_str.html",
            "{{ querystring('a=1&b=2', b=none, c='hello world') }}",
        ),
        (
            "qs_map.html",
            "{{ querystring(base, {'b': none, 'c': 3}) }}",
        ),
        ("qs_empty.html", "{{ querystring('a=1', a=none) }}"),
    ])
    .unwrap();

    assert_eq!(
        engine.render("qs_str.html", ()).unwrap(),
        "?a=1&c=hello%20world"
    );

    let ctx = minijinja::context! {
        base => minijinja::context! { a => "1", b => "2" },
    };
    assert_eq!(engine.render("qs_map.html", &ctx).unwrap(), "?a=1&c=3");
    assert_eq!(engine.render("qs_empty.html", ()).unwrap(), "");
}

#[test]
fn test_render_partial_inline_and_block_fallback() {
    let dir = create_temp_template_dir();
    let tpl = r#"<html><body>
{# partialdef item_row #}<li class="item">{{ name }}</li>{# endpartialdef #}
{% block sidebar %}<aside>{{ section }}</aside>{% endblock %}
</body></html>"#;
    fs::write(dir.path().join("list.html"), tpl).unwrap();

    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    let partial = engine
        .render_partial(
            "list.html",
            "item_row",
            minijinja::context! { name => "<b>Widget</b>" },
        )
        .unwrap();
    assert_eq!(
        partial,
        r#"<li class="item">&lt;b&gt;Widget&lt;&#x2f;b&gt;</li>"#
    );

    let block_partial = engine
        .render_partial(
            "list.html",
            "sidebar",
            minijinja::context! { section => "News" },
        )
        .unwrap();
    assert_eq!(block_partial, "<aside>News</aside>");

    let missing = engine.render_partial("list.html", "nonexistent", ());
    assert!(matches!(
        missing,
        Err(TemplateError::PartialNotFound { .. })
    ));

    // Also works with embedded templates
    let emb = TemplateEngine::from_embedded(&[(
        "emb.html",
        "Header {# partialdef card #}<div>{{ v }}</div>{# endpartialdef #} Footer",
    )])
    .unwrap();
    assert_eq!(
        emb.render_partial("emb.html", "card", minijinja::context! { v => 42 })
            .unwrap(),
        "<div>42</div>"
    );
}

#[test]
fn test_extended_django_filters() {
    let engine = TemplateEngine::from_embedded(&[
        ("json_script.html", "{{ data|json_script('payload') }}"),
        ("timesince.html", "{{ start|timesince(ref_dt) }}"),
        ("timeuntil.html", "{{ future|timeuntil(ref_dt) }}"),
        ("linebreaks.html", "{{ text|linebreaks }}"),
        ("linebreaksbr.html", "{{ text|linebreaksbr }}"),
        ("slugify.html", "{{ text|slugify }}"),
        ("urlize.html", "{{ text|urlize }}"),
        ("striptags.html", "{{ text|striptags }}"),
        ("yesno.html", "{{ val|yesno('yeah,no,maybe') }}"),
        ("ordinal.html", "{{ n|ordinal }}"),
        ("naturalday.html", "{{ d|naturalday }}"),
        ("apnumber.html", "{{ n|apnumber }}"),
        (
            "dictsort.html",
            "{% for item in items|dictsort('age') %}{{ item.name }}:{{ item.age }};{% endfor %}",
        ),
    ])
    .unwrap();

    // json_script escapes <, >, &
    let js = engine
        .render(
            "json_script.html",
            minijinja::context! { data => minijinja::context! { html => "</script>&<tag>" } },
        )
        .unwrap();
    assert_eq!(
        js,
        r#"<script id="payload" type="application/json">{"html":"\u003C/script\u003E\u0026\u003Ctag\u003E"}</script>"#
    );

    // timesince & timeuntil
    assert_eq!(
        engine
            .render(
                "timesince.html",
                minijinja::context! {
                    start => "2026-07-01T10:00:00+00:00",
                    ref_dt => "2026-07-03T13:00:00+00:00",
                }
            )
            .unwrap(),
        "2 days, 3 hours"
    );
    assert_eq!(
        engine
            .render(
                "timeuntil.html",
                minijinja::context! {
                    future => "2026-07-10T10:00:00+00:00",
                    ref_dt => "2026-07-01T10:00:00+00:00",
                }
            )
            .unwrap(),
        "1 week, 2 days"
    );

    // linebreaks & linebreaksbr
    assert_eq!(
        engine
            .render(
                "linebreaks.html",
                minijinja::context! { text => "Hello\nworld\n\nSecond <p>" }
            )
            .unwrap(),
        "<p>Hello<br>world</p>\n\n<p>Second &lt;p&gt;</p>"
    );
    assert_eq!(
        engine
            .render(
                "linebreaksbr.html",
                minijinja::context! { text => "a\nb\nc" }
            )
            .unwrap(),
        "a<br>b<br>c"
    );

    // slugify
    assert_eq!(
        engine
            .render(
                "slugify.html",
                minijinja::context! { text => "  Hello, World! -- Rust_2026  " }
            )
            .unwrap(),
        "hello-world-rust-2026"
    );

    // urlize
    assert_eq!(
        engine
            .render(
                "urlize.html",
                minijinja::context! { text => "Visit https://example.com/a, or www.rust-lang.org." }
            )
            .unwrap(),
        r#"Visit <a href="https://example.com/a" rel="nofollow">https://example.com/a</a>, or <a href="https://www.rust-lang.org" rel="nofollow">www.rust-lang.org</a>."#
    );

    // striptags
    assert_eq!(
        engine
            .render(
                "striptags.html",
                minijinja::context! { text => "<b>Hello</b> <i>world</i>!" }
            )
            .unwrap(),
        "Hello world!"
    );

    // yesno
    assert_eq!(
        engine
            .render("yesno.html", minijinja::context! { val => true })
            .unwrap(),
        "yeah"
    );
    assert_eq!(
        engine
            .render("yesno.html", minijinja::context! { val => false })
            .unwrap(),
        "no"
    );
    assert_eq!(
        engine
            .render("yesno.html", minijinja::context! { val => None::<bool> })
            .unwrap(),
        "maybe"
    );

    // ordinal
    for (n, expected) in [
        (1, "1st"),
        (2, "2nd"),
        (3, "3rd"),
        (4, "4th"),
        (11, "11th"),
        (12, "12th"),
        (13, "13th"),
        (21, "21st"),
    ] {
        assert_eq!(
            engine
                .render("ordinal.html", minijinja::context! { n => n })
                .unwrap(),
            expected
        );
    }

    // naturalday
    let today = chrono::Utc::now().date_naive();
    let yesterday = today - chrono::Duration::days(1);
    let tomorrow = today + chrono::Duration::days(1);
    let other = today - chrono::Duration::days(10);
    assert_eq!(
        engine
            .render(
                "naturalday.html",
                minijinja::context! { d => today.format("%Y-%m-%d").to_string() }
            )
            .unwrap(),
        "today"
    );
    assert_eq!(
        engine
            .render(
                "naturalday.html",
                minijinja::context! { d => yesterday.format("%Y-%m-%d").to_string() }
            )
            .unwrap(),
        "yesterday"
    );
    assert_eq!(
        engine
            .render(
                "naturalday.html",
                minijinja::context! { d => tomorrow.format("%Y-%m-%d").to_string() }
            )
            .unwrap(),
        "tomorrow"
    );
    assert_eq!(
        engine
            .render(
                "naturalday.html",
                minijinja::context! { d => other.format("%Y-%m-%d").to_string() }
            )
            .unwrap(),
        other.format("%Y-%m-%d").to_string()
    );

    // apnumber
    assert_eq!(
        engine
            .render("apnumber.html", minijinja::context! { n => 1 })
            .unwrap(),
        "one"
    );
    assert_eq!(
        engine
            .render("apnumber.html", minijinja::context! { n => 9 })
            .unwrap(),
        "nine"
    );
    assert_eq!(
        engine
            .render("apnumber.html", minijinja::context! { n => 10 })
            .unwrap(),
        "10"
    );

    // dictsort
    let items = vec![
        minijinja::context! { name => "Carol", age => 40 },
        minijinja::context! { name => "Alice", age => 25 },
        minijinja::context! { name => "Bob", age => 30 },
    ];
    assert_eq!(
        engine
            .render("dictsort.html", minijinja::context! { items => items })
            .unwrap(),
        "Alice:25;Bob:30;Carol:40;"
    );
}

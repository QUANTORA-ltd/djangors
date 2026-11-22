use super::*;
use bytes::Bytes as HttpBytes;
use djangors_core::state::AppState;
use djangors_macros::Model;
use hyper::{HeaderMap, Method, Uri};

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_cbv_model")]
pub struct CbvTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 40)]
    pub name: String,
    pub age: i64,
}

// All tests below share the one real `djangors_test` database and a fixed table name
// (the model's `table_name` is fixed at macro-expansion time, so per-test unique naming
// isn't an option the way it is for hand-written SQL). `cargo test` runs `#[tokio::test]`
// functions concurrently by default, so without this lock two tests racing on
// `DROP TABLE IF EXISTS` + `CREATE TABLE` produce a "relation already exists" error -
// confirmed directly (5/5 tests failed with Postgres error 42P07 before this lock was
// added). Every test must acquire this guard before calling `setup()`.
static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn setup() -> (djangors_db::Database, tempfile::TempDir, TemplateEngine) {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database().clone();
    let auto_pk = db.dialect().auto_pk_type();

    db.conn()
        .execute("DROP TABLE IF EXISTS test_cbv_model", &[])
        .await
        .unwrap();
    let create_sql = format!(
        "CREATE TABLE test_cbv_model (
                id {auto_pk},
                name TEXT NOT NULL,
                age BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("list.html"),
        "{% for o in object_list %}{{ o.name }}:{{ o.age }};{% endfor %}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("detail.html"),
        "{{ object.name }}:{{ object.age }}",
    )
    .unwrap();
    std::fs::write(
            dir.path().join("form.html"),
            "{% if errors %}ERRORS:{% for k in errors %}{{ k }}={{ errors[k] }};{% endfor %}{% else %}OK{% endif %}",
        )
        .unwrap();
    std::fs::write(dir.path().join("confirm.html"), "confirm {{ object.name }}").unwrap();
    let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

    (db, dir, engine)
}

fn make_request(method: Method, body: &str, db: djangors_db::Database) -> Request {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    Request::new(
        method,
        Uri::from_static("/"),
        headers,
        HttpBytes::from(body.to_string()),
    )
    .with_state(AppState::new().insert(db))
}

fn path_with_pk(pk: i64) -> PathParams {
    let mut params = PathParams::new();
    params.insert("pk", &pk.to_string());
    params
}

#[tokio::test]
async fn test_list_view_renders_real_rows() {
    let _guard = TEST_DB_LOCK.lock().await;
    let (db, _dir, engine) = setup().await;
    CbvTestModel {
        id: 0,
        name: "Alice".into(),
        age: 30,
    }
    .save(&db)
    .await
    .unwrap();
    CbvTestModel {
        id: 0,
        name: "Bob".into(),
        age: 25,
    }
    .save(&db)
    .await
    .unwrap();

    let config = ViewSetConfig {
        engine: &engine,
        template_name: "list.html",
        success_url: "/",
    };
    let req = make_request(Method::GET, "", db.clone());
    let res = ListView::<CbvTestModel>::list(req, PathParams::new(), &config)
        .await
        .unwrap();
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Alice:30"), "body was: {body}");
    assert!(body.contains("Bob:25"), "body was: {body}");

    db.conn()
        .execute("DROP TABLE test_cbv_model", &[])
        .await
        .ok();
}

#[tokio::test]
async fn test_detail_view_renders_the_matching_row() {
    let _guard = TEST_DB_LOCK.lock().await;
    let (db, _dir, engine) = setup().await;
    let saved = CbvTestModel {
        id: 0,
        name: "Carol".into(),
        age: 40,
    }
    .save(&db)
    .await
    .unwrap();

    let config = ViewSetConfig {
        engine: &engine,
        template_name: "detail.html",
        success_url: "/",
    };
    let req = make_request(Method::GET, "", db.clone());
    let res = DetailView::<CbvTestModel>::detail(req, path_with_pk(saved.id), &config)
        .await
        .unwrap();
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert_eq!(body, "Carol:40");

    db.conn()
        .execute("DROP TABLE test_cbv_model", &[])
        .await
        .ok();
}

#[tokio::test]
async fn test_create_view_post_inserts_a_real_row_and_redirects() {
    let _guard = TEST_DB_LOCK.lock().await;
    let (db, _dir, engine) = setup().await;
    let config = ViewSetConfig {
        engine: &engine,
        template_name: "form.html",
        success_url: "/done",
    };

    // Valid data actually inserts a row and redirects.
    let req = make_request(Method::POST, "name=Dave&age=22", db.clone());
    let res = CreateView::<CbvTestModel>::create(req, PathParams::new(), &config)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(res.headers().get(hyper::header::LOCATION).unwrap(), "/done");
    let ph = db.dialect().placeholder(1);
    let sql = format!("SELECT COUNT(*) FROM test_cbv_model WHERE name = {ph}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::Text("Dave".to_string())])
        .await
        .unwrap();
    let count = row.try_i64(0).unwrap().unwrap();
    assert_eq!(count, 1, "the valid submission must actually insert a row");

    // Invalid data (missing required `name`) does not insert a row and re-renders with errors.
    let req = make_request(Method::POST, "age=22", db.clone());
    let res = CreateView::<CbvTestModel>::create(req, PathParams::new(), &config)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("ERRORS"), "body was: {body}");
    let row_total = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM test_cbv_model", &[])
        .await
        .unwrap();
    let total = row_total.try_i64(0).unwrap().unwrap();
    assert_eq!(total, 1, "invalid submission must not insert any row");

    db.conn()
        .execute("DROP TABLE test_cbv_model", &[])
        .await
        .ok();
}

#[tokio::test]
async fn test_update_view_post_changes_the_existing_row_and_redirects() {
    let _guard = TEST_DB_LOCK.lock().await;
    let (db, _dir, engine) = setup().await;
    let saved = CbvTestModel {
        id: 0,
        name: "Eve".into(),
        age: 50,
    }
    .save(&db)
    .await
    .unwrap();
    let original_id = saved.id;

    let config = ViewSetConfig {
        engine: &engine,
        template_name: "form.html",
        success_url: "/updated",
    };
    let req = make_request(Method::POST, "name=EveUpdated&age=51", db.clone());
    let res = UpdateView::<CbvTestModel>::update(req, path_with_pk(original_id), &config)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get(hyper::header::LOCATION).unwrap(),
        "/updated"
    );

    let ph = db.dialect().placeholder(1);
    let sql = format!("SELECT name, age FROM test_cbv_model WHERE id = {ph}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::I64(original_id)])
        .await
        .unwrap();
    let name = row.try_string_by_name("name").unwrap().unwrap();
    let age = row.try_i64_by_name("age").unwrap().unwrap();
    assert_eq!(name, "EveUpdated");
    assert_eq!(age, 51);

    db.conn()
        .execute("DROP TABLE test_cbv_model", &[])
        .await
        .ok();
}

#[tokio::test]
async fn test_delete_view_post_removes_the_row() {
    let _guard = TEST_DB_LOCK.lock().await;
    let (db, _dir, engine) = setup().await;
    let saved = CbvTestModel {
        id: 0,
        name: "Frank".into(),
        age: 60,
    }
    .save(&db)
    .await
    .unwrap();

    let config = ViewSetConfig {
        engine: &engine,
        template_name: "confirm.html",
        success_url: "/gone",
    };

    // GET renders a confirmation page without deleting anything.
    let req = make_request(Method::GET, "", db.clone());
    let res = DeleteView::<CbvTestModel>::delete(req, path_with_pk(saved.id), &config)
        .await
        .unwrap();
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Frank"), "body was: {body}");

    let ph = db.dialect().placeholder(1);
    let sql = format!("SELECT COUNT(*) FROM test_cbv_model WHERE id = {ph}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::I64(saved.id)])
        .await
        .unwrap();
    let still_there = row.try_i64(0).unwrap().unwrap();
    assert_eq!(still_there, 1, "GET must not delete anything");

    // POST actually deletes the row.
    let req = make_request(Method::POST, "", db.clone());
    let res = DeleteView::<CbvTestModel>::delete(req, path_with_pk(saved.id), &config)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    let row_gone = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::I64(saved.id)])
        .await
        .unwrap();
    let gone = row_gone.try_i64(0).unwrap().unwrap();
    assert_eq!(gone, 0, "POST must actually delete the row");

    db.conn()
        .execute("DROP TABLE test_cbv_model", &[])
        .await
        .ok();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_archive_article")]
pub struct ArchiveArticle {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 80)]
    pub title: String,
    pub published_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn test_template_view_and_redirect_view() {
    let engine = TemplateEngine::from_embedded(&[("about.html", "About {{ org }}")]).unwrap();
    let view = TemplateView::new(&engine, "about.html")
        .with_context(serde_json::json!({"org": "Djangors"}));
    let req = Request::new(
        Method::GET,
        Uri::from_static("/about"),
        HeaderMap::new(),
        HttpBytes::new(),
    );
    let res = view.dispatch(req, PathParams::new()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(res.body().to_vec()).unwrap(),
        "About Djangors"
    );

    // Temporary 302 redirect without query string
    let r_temp = RedirectView::new("/new-home");
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/old?a=1"),
        HeaderMap::new(),
        HttpBytes::new(),
    );
    let res1 = r_temp.dispatch(req1, PathParams::new()).await.unwrap();
    assert_eq!(res1.status(), StatusCode::FOUND);
    assert_eq!(
        res1.headers().get(hyper::header::LOCATION).unwrap(),
        "/new-home"
    );

    // Permanent 301 redirect with query string forwarding
    let r_perm = RedirectView::new("/new-home")
        .permanent(true)
        .query_string(true);
    let req2 = Request::new(
        Method::GET,
        Uri::from_static("/old?a=1&b=2"),
        HeaderMap::new(),
        HttpBytes::new(),
    );
    let res2 = r_perm.dispatch(req2, PathParams::new()).await.unwrap();
    assert_eq!(res2.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        res2.headers().get(hyper::header::LOCATION).unwrap(),
        "/new-home?a=1&b=2"
    );
}

#[tokio::test]
async fn test_form_view_get_and_post() {
    let engine = TemplateEngine::from_embedded(&[(
        "contact.html",
        "{% if errors %}ERR:{{ errors.email }}{% else %}FORM:{{ heading }}{% endif %}",
    )])
    .unwrap();
    let view = FormView::new(&engine, "contact.html", "/thanks")
        .with_context(serde_json::json!({"heading": "Contact Us"}));

    // GET renders form
    let get_req = Request::new(
        Method::GET,
        Uri::from_static("/contact"),
        HeaderMap::new(),
        HttpBytes::new(),
    );
    let get_res = view
        .dispatch(get_req, PathParams::new(), |_| Ok::<(), _>(()))
        .await
        .unwrap();
    assert_eq!(get_res.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(get_res.body().to_vec()).unwrap(),
        "FORM:Contact Us"
    );

    // Invalid POST re-renders with errors
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let bad_req = Request::new(
        Method::POST,
        Uri::from_static("/contact"),
        headers.clone(),
        HttpBytes::from("email=not-an-email"),
    );
    let bad_res = view
        .dispatch(bad_req, PathParams::new(), |data| {
            let email = data.get("email").map(String::as_str).unwrap_or("");
            if email.contains('@') {
                Ok(email.to_string())
            } else {
                let mut errs = HashMap::new();
                errs.insert("email".into(), "Invalid email".into());
                Err(errs)
            }
        })
        .await
        .unwrap();
    assert_eq!(bad_res.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8(bad_res.body().to_vec()).unwrap(),
        "ERR:Invalid email"
    );

    // Valid POST redirects to success_url
    let good_req = Request::new(
        Method::POST,
        Uri::from_static("/contact"),
        headers,
        HttpBytes::from("email=alice%40example.com"),
    );
    let good_res = view
        .dispatch(good_req, PathParams::new(), |data| {
            let email = data.get("email").map(String::as_str).unwrap_or("");
            if email.contains('@') {
                Ok(email.to_string())
            } else {
                Err(HashMap::new())
            }
        })
        .await
        .unwrap();
    assert_eq!(good_res.status(), StatusCode::FOUND);
    assert_eq!(
        good_res.headers().get(hyper::header::LOCATION).unwrap(),
        "/thanks"
    );
}

#[tokio::test]
async fn test_date_based_archive_views() {
    let _guard = TEST_DB_LOCK.lock().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database().clone();
    let auto_pk = db.dialect().auto_pk_type();

    db.conn()
        .execute("DROP TABLE IF EXISTS test_archive_article", &[])
        .await
        .unwrap();
    let create_sql = format!(
        "CREATE TABLE test_archive_article (
            id {auto_pk},
            title TEXT NOT NULL,
            published_at TIMESTAMPTZ NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let mk_dt = |y, m, d, h| {
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(h, 0, 0)
                .unwrap(),
            chrono::Utc,
        )
    };

    for (title, dt) in [
        ("2025 Post", mk_dt(2025, 11, 5, 10)),
        ("Jan Post", mk_dt(2026, 1, 15, 9)),
        ("July Morning", mk_dt(2026, 7, 22, 8)),
        ("July Evening", mk_dt(2026, 7, 22, 18)),
    ] {
        ArchiveArticle {
            id: 0,
            title: title.into(),
            published_at: dt,
        }
        .save(&db)
        .await
        .unwrap();
    }

    let engine = TemplateEngine::from_embedded(&[(
        "archive.html",
        "{% for o in object_list %}{{ o.title }};{% endfor %}",
    )])
    .unwrap();
    let cfg = DateArchiveConfig::new(&engine, "archive.html", "published_at");

    // ArchiveIndexView returns all ordered newest first
    let res_all = ArchiveIndexView::<ArchiveArticle>::list(
        make_request(Method::GET, "", db.clone()),
        PathParams::new(),
        &cfg,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(res_all.body().to_vec()).unwrap(),
        "July Evening;July Morning;Jan Post;2025 Post;"
    );

    // YearArchiveView for 2026
    let mut y_params = PathParams::new();
    y_params.insert("year", "2026");
    let res_year = YearArchiveView::<ArchiveArticle>::list(
        make_request(Method::GET, "", db.clone()),
        y_params,
        &cfg,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(res_year.body().to_vec()).unwrap(),
        "July Evening;July Morning;Jan Post;"
    );

    // MonthArchiveView for 2026-07
    let mut m_params = PathParams::new();
    m_params.insert("year", "2026");
    m_params.insert("month", "7");
    let res_month = MonthArchiveView::<ArchiveArticle>::list(
        make_request(Method::GET, "", db.clone()),
        m_params,
        &cfg,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(res_month.body().to_vec()).unwrap(),
        "July Evening;July Morning;"
    );

    // DayArchiveView for 2026-07-22
    let mut d_params = PathParams::new();
    d_params.insert("year", "2026");
    d_params.insert("month", "7");
    d_params.insert("day", "22");
    let res_day = DayArchiveView::<ArchiveArticle>::list(
        make_request(Method::GET, "", db.clone()),
        d_params,
        &cfg,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(res_day.body().to_vec()).unwrap(),
        "July Evening;July Morning;"
    );

    // allow_empty(false) returns NotFound on empty year
    let strict_cfg =
        DateArchiveConfig::new(&engine, "archive.html", "published_at").allow_empty(false);
    let mut empty_params = PathParams::new();
    empty_params.insert("year", "2020");
    let empty_res = YearArchiveView::<ArchiveArticle>::list(
        make_request(Method::GET, "", db.clone()),
        empty_params,
        &strict_cfg,
    )
    .await;
    assert!(matches!(empty_res, Err(DjangorsError::NotFound)));

    db.conn()
        .execute("DROP TABLE test_archive_article", &[])
        .await
        .ok();
}

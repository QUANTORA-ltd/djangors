use super::*;
use hyper::Request;
use tower::{util::service_fn, ServiceExt};

#[test]
fn translate_falls_back_to_default_then_id() {
    let mut locales = Locales::new("en-US");
    locales.add_locale("en-US", "hello = Hello").unwrap();
    locales.add_locale("fr-FR", "hello = Bonjour").unwrap();
    assert_eq!(locales.translate("fr-FR", "hello", None), "Bonjour");
    assert_eq!(locales.translate("de-DE", "hello", None), "Hello");
    assert_eq!(locales.translate("de-DE", "missing", None), "missing");
}

#[tokio::test]
async fn layer_reads_header_and_defaults() {
    let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
    }));
    let response = service
        .oneshot(
            Request::builder()
                .header("accept-language", "fr-FR, en;q=0.8")
                .body(())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response, ResolvedLocale("fr-FR".into()));

    let session = djangors_sessions::Session::new_empty();
    session.set("_locale", "de-DE");
    let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
    }));
    let response = service
        .oneshot(
            Request::builder()
                .header("accept-language", "not a locale")
                .extension(session)
                .body(())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response, ResolvedLocale("de-DE".into()));

    let service = LocaleLayer::new("en-US").layer(service_fn(|req: Request<()>| async move {
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ResolvedLocale>().unwrap().clone())
    }));
    let response = service.oneshot(Request::new(())).await.unwrap();
    assert_eq!(response, ResolvedLocale("en-US".into()));
}

#[test]
fn trans_filter_and_date_are_localized() {
    let mut locales = Locales::new("en-US");
    locales.add_locale("en-US", "hello = Hello").unwrap();
    locales.add_locale("fr-FR", "hello = Bonjour").unwrap();
    let value = locales_value(Arc::new(locales));
    assert_eq!(
        trans("hello".into(), value.clone(), "en-US".into()).unwrap(),
        "Hello"
    );
    assert_eq!(
        trans("hello".into(), value, "fr-FR".into()).unwrap(),
        "Bonjour"
    );
    let date = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
    assert_eq!(localized_date(date, "en-US"), "07/22/2026");
    assert_eq!(localized_date(date, "en-GB"), "22/07/2026");
}

#[test]
fn l10n_number_date_and_datetime_formatting() {
    assert_eq!(format_number(1_234_567.89, "en-US"), "1,234,567.89");
    assert_eq!(format_number(-12_345.0, "en-US"), "-12,345");
    assert_eq!(format_number(1_234_567.89, "de-DE"), "1.234.567,89");
    assert_eq!(format_number(1_234_567.89, "es-ES"), "1.234.567,89");
    assert_eq!(
        format_number(1_234_567.89, "fr-FR"),
        "1\u{00A0}234\u{00A0}567,89"
    );

    let date = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
    assert_eq!(format_date(date, "en-US"), "07/22/2026");
    assert_eq!(format_date(date, "de-DE"), "22.07.2026");
    assert_eq!(format_date(date, "fr-FR"), "22/07/2026");
    assert_eq!(format_date(date, "ja-JP"), "2026-07-22");

    let dt = DateTime::<Utc>::from_naive_utc_and_offset(date.and_hms_opt(14, 30, 5).unwrap(), Utc);
    assert_eq!(format_datetime(dt, "en-US"), "07/22/2026 14:30:05");
    assert_eq!(format_datetime(dt, "de-DE"), "22.07.2026 14:30:05");
    assert_eq!(format_datetime(dt, "fr-FR"), "22/07/2026 14:30:05");
}

#[tokio::test]
async fn per_request_timezone_context() {
    assert_eq!(current_timezone(), None);

    let inner = with_timezone("America/New_York", async {
        assert_eq!(current_timezone().as_deref(), Some("America/New_York"));
        activate_timezone("Europe/Paris");
        assert_eq!(current_timezone().as_deref(), Some("Europe/Paris"));
        let nested = with_timezone("Asia/Tokyo", async {
            assert_eq!(current_timezone().as_deref(), Some("Asia/Tokyo"));
            current_timezone()
        })
        .await;
        assert_eq!(nested.as_deref(), Some("Asia/Tokyo"));
        assert_eq!(current_timezone().as_deref(), Some("Europe/Paris"));
        current_timezone()
    })
    .await;

    assert_eq!(inner.as_deref(), Some("Europe/Paris"));
    assert_eq!(current_timezone(), None);

    // Sync fallback via activate_timezone / deactivate_timezone
    activate_timezone("UTC");
    assert_eq!(current_timezone().as_deref(), Some("UTC"));
    deactivate_timezone();
    assert_eq!(current_timezone(), None);
}

#[test]
fn detect_locale_and_i18n_patterns() {
    let supported = ["en", "fr", "de"];
    // URL prefix has highest priority
    assert_eq!(
        detect_locale(Some("de-DE"), Some("en"), Some("fr"), &supported, "en"),
        "fr"
    );
    // Cookie overrides Accept-Language
    assert_eq!(
        detect_locale(Some("de-DE"), Some("fr-CA"), None, &supported, "en"),
        "fr"
    );
    // Accept-Language honors q-value weights
    assert_eq!(
        detect_locale(
            Some("es;q=0.9, de-AT;q=0.8, fr;q=0.95"),
            None,
            None,
            &supported,
            "en"
        ),
        "fr"
    );
    // Fallback to default
    assert_eq!(
        detect_locale(Some("ja-JP"), None, None, &supported, "en"),
        "en"
    );

    let prefixed = i18n_patterns(&["en", "fr"], &["/", "/about"], true, Some("en"));
    assert_eq!(
        prefixed,
        vec![
            ("/en/".into(), "/".into()),
            ("/en/about".into(), "/about".into()),
            ("/fr/".into(), "/".into()),
            ("/fr/about".into(), "/about".into()),
        ]
    );

    let unprefixed_default = i18n_patterns(&["en", "fr"], &["/about"], false, Some("en"));
    assert_eq!(
        unprefixed_default,
        vec![
            ("/about".into(), "/about".into()),
            ("/fr/about".into(), "/about".into()),
        ]
    );
}

#[tokio::test]
async fn locale_layer_sets_active_locale_from_prefix_and_cookie() {
    let layer =
        LocaleLayer::new("en").with_supported_locales(vec!["en".into(), "fr".into(), "de".into()]);

    let svc = layer
        .clone()
        .layer(service_fn(|req: Request<()>| async move {
            Ok::<_, std::convert::Infallible>(
                req.extensions().get::<ActiveLocale>().unwrap().clone(),
            )
        }));
    let resp = svc
        .oneshot(
            Request::builder()
                .uri("/fr/articles")
                .header("accept-language", "de-DE")
                .body(())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp, ActiveLocale("fr".into()));

    let svc2 = layer.layer(service_fn(|req: Request<()>| async move {
        Ok::<_, std::convert::Infallible>(req.extensions().get::<ActiveLocale>().unwrap().clone())
    }));
    let resp2 = svc2
        .oneshot(
            Request::builder()
                .uri("/articles")
                .header("cookie", "djangors_language=de")
                .body(())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2, ActiveLocale("de".into()));
}

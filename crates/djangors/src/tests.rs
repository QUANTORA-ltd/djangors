#[test]
fn framework_modules_expose_public_items() {
    let _ = std::mem::size_of::<crate::core::Router>();
    let _ = std::mem::size_of::<crate::orm::OrmError>();
    let _ = std::mem::size_of::<crate::db::DatabaseConfig>();
    #[cfg(feature = "migrations")]
    let _ = std::mem::size_of::<crate::migrations::Operation>();
    #[cfg(feature = "rest")]
    let _ = std::mem::size_of::<crate::rest::AllowAny>();
    #[cfg(feature = "admin")]
    let _ = std::mem::size_of::<crate::admin::ModelAdminConfig>();
    #[cfg(feature = "auth")]
    let _ = std::mem::size_of::<crate::auth::ModelBackend>();
    #[cfg(feature = "forms")]
    let _ = std::mem::size_of::<crate::forms::CharField>();
    #[cfg(feature = "sessions")]
    let _ = std::mem::size_of::<crate::sessions::Session>();
    #[cfg(feature = "template")]
    let _ = std::mem::size_of::<crate::template::TemplateEngine>();
    #[cfg(feature = "staticfiles")]
    let _ = std::mem::size_of::<crate::staticfiles::StaticFiles>();
    #[cfg(feature = "cache")]
    let _ = std::mem::size_of::<crate::cache::InMemoryCache>();
    #[cfg(feature = "mail")]
    let _ = std::mem::size_of::<crate::mail::ConsoleBackend>();
    #[cfg(feature = "i18n")]
    let _ = std::mem::size_of::<crate::i18n::Catalog>();
}

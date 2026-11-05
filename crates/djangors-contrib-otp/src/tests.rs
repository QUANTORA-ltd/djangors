use super::*;
#[cfg(feature = "models")]
use djangors_orm::Model;

#[test]
fn generated_code_verifies_and_invalid_code_does_not() {
    let secret = generate_secret();
    let bytes = Secret::Encoded(secret.clone()).to_bytes().unwrap();
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, String::new()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = totp.generate(now);
    assert!(verify_code(&secret, &code));
    assert!(!verify_code(&secret, "000000"));
}

#[test]
fn provisioning_uri_has_expected_fields() {
    let uri = provisioning_uri(&generate_secret(), "alice@example.com", "Djangors");
    assert!(uri.starts_with("otpauth://totp/"));
    assert!(uri.contains("alice%40example.com"));
    assert!(uri.contains("issuer=Djangors"));
}

#[tokio::test]
#[cfg(feature = "models")]
async fn enrollment_round_trip_against_database() {
    let Ok(db) = djangors_test::TestDatabase::connect().await else {
        return;
    };
    let pk_type = db.database().dialect().auto_pk_type();
    db.create_table(&format!(
            "CREATE TABLE IF NOT EXISTS djangors_otp_device (id {pk_type}, \"user\" BIGINT NOT NULL, secret VARCHAR(255) NOT NULL, confirmed BOOLEAN NOT NULL)"
        )).await.unwrap();
    let user_id = 42;
    let device = OtpDevice {
        id: 0,
        user: ForeignKey::new(user_id),
        secret: generate_secret(),
        confirmed: false,
    }
    .save(db.database())
    .await
    .unwrap();
    assert!(!device.confirmed);
    OtpDevice::objects()
        .filter(djangors_orm::q!(id = device.id))
        .unwrap()
        .update(db.database(), djangors_orm::set!(confirmed = true))
        .await
        .unwrap();
    let loaded = OtpDevice::objects()
        .filter(djangors_orm::q!(id = device.id))
        .unwrap()
        .first(db.database())
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.confirmed);
    db.drop_table("djangors_otp_device").await.unwrap();
}

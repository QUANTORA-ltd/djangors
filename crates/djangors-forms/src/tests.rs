use super::*;
use std::collections::HashMap;

#[test]
fn test_char_field() {
    // required + present
    let field = CharField {
        max_length: None,
        required: true,
    };
    assert_eq!(field.clean(Some("hello")), Ok("hello".to_string()));

    // required + absent
    assert_eq!(
        field.clean(None),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );

    // required + empty string
    assert_eq!(
        field.clean(Some("")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );

    // not-required + absent
    let optional_field = CharField {
        max_length: None,
        required: false,
    };
    assert_eq!(optional_field.clean(None), Ok("".to_string()));
    assert_eq!(optional_field.clean(Some("")), Ok("".to_string()));

    // max_length respected
    let limit_field = CharField {
        max_length: Some(5),
        required: true,
    };
    assert_eq!(limit_field.clean(Some("abcde")), Ok("abcde".to_string()));
    assert_eq!(
        limit_field.clean(Some("abcdef")),
        Err(FieldError(vec![
            "Ensure this value has at most 5 characters.".to_string()
        ]))
    );

    // no trimming happens
    let whitespace_field = CharField {
        max_length: None,
        required: false,
    };
    assert_eq!(
        whitespace_field.clean(Some("  hello  ")),
        Ok("  hello  ".to_string())
    );
}

#[test]
fn test_integer_field() {
    let field = IntegerField {
        min: Some(5),
        max: Some(10),
        required: true,
    };

    // valid integer parses correctly
    assert_eq!(field.clean(Some("7")), Ok(Some(7)));

    // boundary values accepted (inclusive)
    assert_eq!(field.clean(Some("5")), Ok(Some(5)));
    assert_eq!(field.clean(Some("10")), Ok(Some(10)));

    // below min
    assert_eq!(
        field.clean(Some("4")),
        Err(FieldError(vec![
            "Ensure this value is greater than or equal to 5.".to_string()
        ]))
    );

    // above max
    assert_eq!(
        field.clean(Some("11")),
        Err(FieldError(vec![
            "Ensure this value is less than or equal to 10.".to_string()
        ]))
    );

    // non-numeric string errors clearly
    assert_eq!(
        field.clean(Some("abc")),
        Err(FieldError(vec!["Enter a whole number.".to_string()]))
    );

    // required / not-required absent branching
    assert_eq!(
        field.clean(None),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );

    let optional_field = IntegerField {
        min: None,
        max: None,
        required: false,
    };
    assert_eq!(optional_field.clean(None), Ok(None));
    assert_eq!(optional_field.clean(Some("")), Ok(None));
}

#[test]
fn test_boolean_field() {
    let required_field = BooleanField { required: true };
    let optional_field = BooleanField { required: false };

    // absent -> Ok(false) specifically (not required)
    assert_eq!(optional_field.clean(None), Ok(false));

    // absent + required:true -> Err (since false fails the required check)
    assert_eq!(
        required_field.clean(None),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );

    // each of "on"/"true"/"1" -> Ok(true)
    assert_eq!(optional_field.clean(Some("on")), Ok(true));
    assert_eq!(optional_field.clean(Some("true")), Ok(true));
    assert_eq!(optional_field.clean(Some("1")), Ok(true));

    // "false"/"0"/"" -> Ok(false)
    assert_eq!(optional_field.clean(Some("false")), Ok(false));
    assert_eq!(optional_field.clean(Some("0")), Ok(false));
    assert_eq!(optional_field.clean(Some("")), Ok(false));
    assert_eq!(optional_field.clean(Some("anything_else")), Ok(false));

    // required:true + truthy -> Ok(true)
    assert_eq!(required_field.clean(Some("on")), Ok(true));
    assert_eq!(required_field.clean(Some("true")), Ok(true));
    assert_eq!(required_field.clean(Some("1")), Ok(true));

    // required:true + falsy -> Err
    assert_eq!(
        required_field.clean(Some("false")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );
    assert_eq!(
        required_field.clean(Some("0")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );
    assert_eq!(
        required_field.clean(Some("")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );
}

#[test]
fn test_email_field() {
    let field = EmailField { required: true };

    // valid shapes accepted
    assert_eq!(
        field.clean(Some("test@example.com")),
        Ok("test@example.com".to_string())
    );
    assert_eq!(field.clean(Some("a@b.c")), Ok("a@b.c".to_string()));
    assert_eq!(
        field.clean(Some("first.last@sub.domain.co.uk")),
        Ok("first.last@sub.domain.co.uk".to_string())
    );

    // invalid shapes rejected
    assert_eq!(
        field.clean(Some("noatsign")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("@domain.com")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("user@")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("user@.com")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("user@domain.")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("user@domain..com")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("user@one@two.com")),
        Err(FieldError(vec!["Enter a valid email address.".to_string()]))
    );

    // required / not-required absent branching
    assert_eq!(
        field.clean(None),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );
    assert_eq!(
        field.clean(Some("")),
        Err(FieldError(vec!["This field is required.".to_string()]))
    );

    let optional_field = EmailField { required: false };
    assert_eq!(optional_field.clean(None), Ok("".to_string()));
    assert_eq!(optional_field.clean(Some("")), Ok("".to_string()));
}

#[test]
fn test_form_errors() {
    let mut errors = FormErrors::new();
    assert!(errors.is_empty());

    errors.add_field_error("name", "Name is too short.");
    assert!(!errors.is_empty());
    assert_eq!(
        errors.fields.get("name"),
        Some(&FieldError(vec!["Name is too short.".to_string()]))
    );

    errors.add_field_error("name", "Name cannot contain digits.");
    assert_eq!(
        errors.fields.get("name"),
        Some(&FieldError(vec![
            "Name is too short.".to_string(),
            "Name cannot contain digits.".to_string()
        ]))
    );

    errors.add_non_field_error("Passwords do not match.");
    assert_eq!(
        errors.non_field,
        vec!["Passwords do not match.".to_string()]
    );
}

// --- NEW TESTS: Widgets ---

#[test]
fn test_widget_text_input() {
    let widget = TextInput;
    let attrs = WidgetAttrs::from([("class", "form-control")]);
    let html = widget.render("username", Some("john_doe"), &attrs);
    assert_eq!(
        html,
        r#"<input type="text" name="username" value="john_doe" id="id_username" class="form-control">"#
    );

    let html_no_val = widget.render("username", None, &WidgetAttrs::new());
    assert_eq!(
        html_no_val,
        r#"<input type="text" name="username" id="id_username">"#
    );
}

#[test]
fn test_widget_textarea() {
    let widget = Textarea;
    let attrs = WidgetAttrs::from([("rows", "4")]);
    let html = widget.render("bio", Some("Hello world\nSecond line"), &attrs);
    assert_eq!(
        html,
        "<textarea name=\"bio\" id=\"id_bio\" rows=\"4\">Hello world\nSecond line</textarea>"
    );
}

#[test]
fn test_widget_number_input() {
    let widget = NumberInput;
    let html = widget.render("age", Some("25"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<input type="number" name="age" value="25" id="id_age">"#
    );
}

#[test]
fn test_widget_checkbox_input() {
    let widget = CheckboxInput;
    let html_checked = widget.render("agree", Some("true"), &WidgetAttrs::new());
    assert_eq!(
        html_checked,
        r#"<input type="checkbox" name="agree" checked id="id_agree">"#
    );

    let html_on = widget.render("agree", Some("on"), &WidgetAttrs::new());
    assert_eq!(
        html_on,
        r#"<input type="checkbox" name="agree" checked id="id_agree">"#
    );

    let html_unchecked = widget.render("agree", Some("false"), &WidgetAttrs::new());
    assert_eq!(
        html_unchecked,
        r#"<input type="checkbox" name="agree" id="id_agree">"#
    );
    assert!(!html_unchecked.contains("checked="));
}

#[test]
fn test_widget_select() {
    let choices = vec![
        ("us".to_string(), "United States".to_string()),
        ("ca".to_string(), "Canada".to_string()),
    ];
    let widget = Select::new(choices);
    let html = widget.render("country", Some("ca"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<select name="country" id="id_country"><option value="us">United States</option><option value="ca" selected>Canada</option></select>"#
    );
}

#[test]
fn test_widget_radio_select() {
    let choices = vec![
        ("small".to_string(), "Small".to_string()),
        ("large".to_string(), "Large".to_string()),
    ];
    let widget = RadioSelect::new(choices);
    let html = widget.render("size", Some("large"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<div id="id_size"><div><label for="id_size_0"><input type="radio" name="size" value="small" id="id_size_0"> Small</label></div><div><label for="id_size_1"><input type="radio" name="size" value="large" id="id_size_1" checked> Large</label></div></div>"#
    );
}

#[test]
fn test_widget_date_input() {
    let widget = DateInput;
    let html = widget.render("dob", Some("2026-07-31"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<input type="date" name="dob" value="2026-07-31" id="id_dob">"#
    );
}

#[test]
fn test_widget_email_input() {
    let widget = EmailInput;
    let html = widget.render("email", Some("user@example.com"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<input type="email" name="email" value="user@example.com" id="id_email">"#
    );
}

#[test]
fn test_widget_password_input() {
    let widget_default = PasswordInput::new();
    let html_default = widget_default.render("password", Some("secret123"), &WidgetAttrs::new());
    assert_eq!(
        html_default,
        r#"<input type="password" name="password" id="id_password">"#
    );
    assert!(!html_default.contains("secret123"));

    let widget_show = PasswordInput { render_value: true };
    let html_show = widget_show.render("password", Some("secret123"), &WidgetAttrs::new());
    assert_eq!(
        html_show,
        r#"<input type="password" name="password" value="secret123" id="id_password">"#
    );
}

#[test]
fn test_widget_hidden_input() {
    let widget = HiddenInput;
    let html = widget.render("csrf", Some("token123"), &WidgetAttrs::new());
    assert_eq!(
        html,
        r#"<input type="hidden" name="csrf" value="token123" id="id_csrf">"#
    );
}

#[test]
fn test_dedicated_xss_prevention() {
    let payload = "\"><script>alert(1)</script>";
    let esc_payload = "&quot;&gt;&lt;script&gt;alert(1)&lt;&#x2F;script&gt;";

    // TextInput with payload as name, value, attribute
    let text = TextInput;
    let mut attrs = WidgetAttrs::new();
    attrs.set("data-xss", payload);
    let rendered_text = text.render(payload, Some(payload), &attrs);
    assert!(!rendered_text.contains("<script>"));
    assert!(rendered_text.contains(esc_payload));

    // Select with payload as name, value, option value, option label
    let choices = vec![(payload.to_string(), payload.to_string())];
    let select = Select::new(choices);
    let rendered_select = select.render(payload, Some(payload), &WidgetAttrs::new());
    assert!(!rendered_select.contains("<script>"));
    assert!(rendered_select.contains(esc_payload));

    // RadioSelect
    let radio = RadioSelect::new(vec![(payload.to_string(), payload.to_string())]);
    let rendered_radio = radio.render(payload, Some(payload), &WidgetAttrs::new());
    assert!(!rendered_radio.contains("<script>"));

    // Renderers: BoundField errors, label, help_text
    let bf = BoundField::new(payload, &text)
        .with_value(Some(payload))
        .with_label(payload)
        .with_help_text(payload)
        .with_errors(vec![payload.to_string()]);

    let div_html = as_div(&[bf], &[payload.to_string()]);
    assert!(!div_html.contains("<script>"));
    assert!(div_html.contains(esc_payload));
}

#[test]
fn test_renderers_as_div_as_p_as_table() {
    let widget = TextInput;
    let bf = BoundField::new("first_name", &widget)
        .with_value(Some("John"))
        .with_help_text("Enter your first name")
        .with_errors(vec!["Invalid first name".to_string()]);

    let div = as_div(&[bf], &["Non-field error".to_string()]);
    assert!(div.contains("<ul class=\"errorlist nonfield\"><li>Non-field error</li></ul>"));
    assert!(div.contains("<ul class=\"errorlist\"><li>Invalid first name</li></ul>"));
    assert!(div.contains("<label for=\"id_first_name\">First name</label>"));
    assert!(div
        .contains("<input type=\"text\" name=\"first_name\" value=\"John\" id=\"id_first_name\">"));
    assert!(div.contains("<span class=\"helptext\">Enter your first name</span>"));

    let widget2 = TextInput;
    let bf2 = BoundField::new("age", &widget2);
    let p = as_p(&[bf2], &[]);
    assert!(p.contains("<p><label for=\"id_age\">Age</label>: <input type=\"text\" name=\"age\" id=\"id_age\"></p>"));

    let widget3 = TextInput;
    let bf3 = BoundField::new("email", &widget3);
    let tbl = as_table(&[bf3], &[]);
    assert!(tbl.contains("<tr><th><label for=\"id_email\">Email</label>:</th><td><input type=\"text\" name=\"email\" id=\"id_email\"></td></tr>"));
}

#[test]
fn test_formsets_basic_and_prefixing() {
    let formset: FormSet<()> = FormSet::new()
        .with_prefix("author")
        .with_counts(2, 1)
        .with_can_delete(true);

    assert_eq!(formset.add_prefix(0, "name"), "author-0-name");
    assert_eq!(formset.add_prefix(1, "title"), "author-1-title");

    let mgmt_html = formset.render_management_form();
    assert!(mgmt_html.contains("name=\"author-TOTAL_FORMS\""));
    assert!(mgmt_html.contains("value=\"2\""));
    assert!(mgmt_html.contains("name=\"author-INITIAL_FORMS\""));
    assert!(mgmt_html.contains("value=\"1\""));

    let del_html = formset.render_delete_checkbox(0, false);
    assert!(del_html.contains("name=\"author-0-DELETE\""));

    // Demultiplexing POST data
    let mut post = HashMap::new();
    post.insert("author-TOTAL_FORMS".to_string(), "2".to_string());
    post.insert("author-INITIAL_FORMS".to_string(), "0".to_string());
    post.insert("author-0-name".to_string(), "Alice".to_string());
    post.insert("author-1-name".to_string(), "Bob".to_string());
    post.insert("author-1-DELETE".to_string(), "on".to_string());

    let results = formset
        .clean_with(&post, |map| {
            let name = map.get("name").cloned().unwrap_or_default();
            if name.is_empty() {
                let mut errs = FormErrors::new();
                errs.add_field_error("name", "Name required");
                Err(errs)
            } else {
                Ok(name)
            }
        })
        .expect("formset clean success");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].cleaned, "Alice");
    assert!(!results[0].delete);

    assert_eq!(results[1].cleaned, "Bob");
    assert!(results[1].delete);
}

#[test]
fn test_formset_security_huge_total_forms_rejected() {
    let formset: FormSet<()> = FormSet::new().with_max_num(100);

    let mut malicious_post = HashMap::new();
    // Client sends an attacker-controlled massive TOTAL_FORMS
    malicious_post.insert("form-TOTAL_FORMS".to_string(), "10000000".to_string());
    malicious_post.insert("form-INITIAL_FORMS".to_string(), "0".to_string());

    let res = formset.clean_with(&malicious_post, |_map| Ok("dummy".to_string()));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.management_errors.len(), 1);
    assert!(err.management_errors[0].contains("exceeds maximum allowed"));
    // Confirm form_errors is empty and no allocation occurred
    assert!(err.form_errors.is_empty());
}

#[test]
fn test_honeypot_min_submit_time_and_bot_guard() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let hp = HoneypotField::default();
    assert!(hp.validate(None).is_ok());
    assert!(hp.validate(Some("")).is_ok());
    assert_eq!(
        hp.validate(Some("spam")),
        Err(HoneypotError::HoneypotFilled)
    );
    assert!(hp.render().contains("name=\"website\""));

    let min_time = MinSubmitTime::new(b"secret-key", Duration::from_secs(3));
    let immediate = min_time.generate_token();
    assert_eq!(
        min_time.validate(Some(&immediate)),
        Err(HoneypotError::SubmittedTooFast)
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let old_token = min_time.sign_timestamp(now.saturating_sub(10));
    assert!(min_time.validate(Some(&old_token)).is_ok());
    assert_eq!(
        min_time.validate(Some("12345.bad_sig")),
        Err(HoneypotError::InvalidTimestamp)
    );

    let guard = BotGuard::new(b"secret-key", Duration::from_secs(3));
    assert_eq!(
        guard.evaluate(Some(""), Some(&old_token), Some("Mozilla/5.0")),
        SubmissionCheck::Clean
    );
    assert!(matches!(
        guard.evaluate(Some("bot-value"), Some(&old_token), Some("curl/8.0")),
        SubmissionCheck::Suspicious {
            show_captcha: true,
            ..
        }
    ));
}

#[cfg(feature = "captcha")]
#[tokio::test]
async fn test_captcha_always_pass_fail_and_field() {
    assert!(AlwaysPass.verify("tok", None).await.unwrap().success);
    assert!(!AlwaysFail.verify("tok", None).await.unwrap().success);

    let field = CaptchaField::new(CaptchaProvider::Turnstile, "site-123");
    let html = field.render();
    assert!(html.contains("challenges.cloudflare.com/turnstile"));
    assert!(html.contains("cf-turnstile-response"));
}

// --- NEW TESTS: Standard Django Form Fields ---

#[test]
fn test_float_field() {
    let field = FloatField {
        min_value: Some(0.5),
        max_value: Some(10.5),
        ..FloatField::new()
    };
    assert_eq!(field.clean(Some("4.25")), Ok(Some(4.25)));
    assert_eq!(field.clean(Some("0.5")), Ok(Some(0.5)));
    assert_eq!(field.clean(Some("10.5")), Ok(Some(10.5)));
    assert!(field.clean(Some("0.49")).is_err());
    assert!(field.clean(Some("10.51")).is_err());
    assert!(field.clean(Some("NaN")).is_err());
    assert!(field.clean(Some("inf")).is_err());
    assert!(field.clean(Some("abc")).is_err());
    assert!(field.clean(None).is_err());

    let optional = FloatField {
        required: false,
        ..FloatField::new()
    };
    assert_eq!(optional.clean(None), Ok(None));
    assert_eq!(optional.clean(Some("")), Ok(None));
}

#[test]
fn test_decimal_field() {
    let field = DecimalField {
        max_digits: Some(5),
        decimal_places: Some(2),
        min_value: Some(-10.0),
        max_value: Some(500.0),
        ..DecimalField::new()
    };
    assert_eq!(field.clean(Some("123.45")), Ok(Some("123.45".to_string())));
    assert_eq!(field.clean(Some("0.50")), Ok(Some("0.50".to_string())));
    assert_eq!(field.clean(Some("-9.99")), Ok(Some("-9.99".to_string())));
    assert!(field.clean(Some("12.345")).is_err()); // too many decimal places
    assert!(field.clean(Some("1234.5")).is_err()); // too many digits before decimal
    assert!(field.clean(Some("123456")).is_err()); // too many total digits
    assert!(field.clean(Some("-10.01")).is_err()); // below min
    assert!(field.clean(Some("500.01")).is_err()); // above max
    assert!(field.clean(Some("12.3.4")).is_err()); // malformed
    assert!(field.clean(Some("abc")).is_err());
    assert!(field.clean(None).is_err());

    let optional = DecimalField {
        required: false,
        ..DecimalField::new()
    };
    assert_eq!(optional.clean(None), Ok(None));
}

#[test]
fn test_date_field() {
    let field = DateField::new();
    assert_eq!(
        field.clean(Some("2024-02-29")),
        Ok(Some("2024-02-29".to_string()))
    ); // leap year
    assert!(field.clean(Some("2023-02-29")).is_err()); // non-leap year
    assert!(field.clean(Some("2024-13-01")).is_err()); // invalid month
    assert!(field.clean(Some("2024-04-31")).is_err()); // April has 30 days
    assert!(field.clean(Some("not-a-date")).is_err());
    assert!(field.clean(None).is_err());

    let custom_fmt = DateField {
        input_formats: vec!["%m/%d/%Y".to_string(), "%Y-%m-%d".to_string()],
        required: false,
        ..DateField::new()
    };
    assert_eq!(
        custom_fmt.clean(Some("07/04/2026")),
        Ok(Some("2026-07-04".to_string()))
    );
    assert_eq!(custom_fmt.clean(None), Ok(None));
}

#[test]
fn test_datetime_and_time_fields() {
    let dt = DateTimeField::new();
    assert_eq!(
        dt.clean(Some("2026-07-31 14:30:00")),
        Ok(Some("2026-07-31 14:30:00".to_string()))
    );
    assert_eq!(
        dt.clean(Some("2026-07-31T14:30")),
        Ok(Some("2026-07-31T14:30".to_string()))
    );
    assert_eq!(
        dt.clean(Some("2026-07-31T14:30:45.123Z")),
        Ok(Some("2026-07-31T14:30:45.123Z".to_string()))
    );
    assert_eq!(
        dt.clean(Some("2026-07-31T14:30:45+02:00")),
        Ok(Some("2026-07-31T14:30:45+02:00".to_string()))
    );
    assert!(dt.clean(Some("2026-02-30T14:30:00")).is_err());
    assert!(dt.clean(Some("2026-07-31T25:00:00")).is_err());
    assert!(dt.clean(Some("2026-07-31")).is_err());

    let t = TimeField::new();
    assert_eq!(t.clean(Some("08:15")), Ok(Some("08:15".to_string())));
    assert_eq!(
        t.clean(Some("23:59:59.500")),
        Ok(Some("23:59:59.500".to_string()))
    );
    assert!(t.clean(Some("24:00")).is_err());
    assert!(t.clean(Some("12:60")).is_err());
    assert!(t.clean(Some("invalid")).is_err());
}

#[test]
fn test_duration_field() {
    use std::time::Duration;

    let field = DurationField::new();
    assert_eq!(field.clean(Some("90")), Ok(Some(Duration::from_secs(90))));
    assert_eq!(
        field.clean(Some("1.5")),
        Ok(Some(Duration::from_millis(1500)))
    );
    assert_eq!(
        field.clean(Some("01:30:15")),
        Ok(Some(Duration::from_secs(3600 + 1800 + 15)))
    );
    assert_eq!(
        field.clean(Some("2 03:04:05")),
        Ok(Some(Duration::from_secs(2 * 86400 + 3 * 3600 + 4 * 60 + 5)))
    );
    assert_eq!(
        field.clean(Some("PT1H30M15S")),
        Ok(Some(Duration::from_secs(3600 + 1800 + 15)))
    );
    assert_eq!(
        field.clean(Some("P1DT2H")),
        Ok(Some(Duration::from_secs(86400 + 7200)))
    );
    assert!(field.clean(Some("P")).is_err());
    assert!(field.clean(Some("PT")).is_err());
    assert!(field.clean(Some("-5")).is_err());
    assert!(field.clean(Some("01:99:00")).is_err());
}

#[test]
fn test_url_slug_and_uuid_fields() {
    let url = UrlField {
        max_length: Some(50),
        ..UrlField::new()
    };
    assert_eq!(
        url.clean(Some("https://example.com/path?a=1")),
        Ok("https://example.com/path?a=1".to_string())
    );
    assert_eq!(
        url.clean(Some("http://localhost:8000/admin")),
        Ok("http://localhost:8000/admin".to_string())
    );
    assert!(url.clean(Some("ftp://example.com")).is_err());
    assert!(url.clean(Some("https://bad host.com")).is_err());
    assert!(url
        .clean(Some(
            "https://example.com/very-long-path-exceeding-fifty-characters-easily"
        ))
        .is_err());

    let slug = SlugField {
        max_length: Some(20),
        ..SlugField::new()
    };
    assert_eq!(
        slug.clean(Some("my-post_123")),
        Ok("my-post_123".to_string())
    );
    assert!(slug.clean(Some("has spaces")).is_err());
    assert!(slug.clean(Some("no/slashes")).is_err());
    assert!(slug.clean(Some("this-slug-is-way-too-long")).is_err());

    let uuid = UuidField::new();
    assert_eq!(
        uuid.clean(Some("550E8400-E29B-41D4-A716-446655440000")),
        Ok(Some("550e8400-e29b-41d4-a716-446655440000".to_string()))
    );
    assert_eq!(
        uuid.clean(Some("550e8400e29b41d4a716446655440000")),
        Ok(Some("550e8400-e29b-41d4-a716-446655440000".to_string()))
    );
    assert!(uuid.clean(Some("not-a-uuid")).is_err());
    assert!(uuid
        .clean(Some("550e8400-e29b-41d4-a716-44665544000z"))
        .is_err());
}

#[test]
fn test_file_and_image_fields() {
    let file_field = FileField {
        max_length: Some(30),
        allowed_extensions: vec!["pdf".to_string(), "txt".to_string()],
        ..FileField::new()
    };
    assert_eq!(
        file_field.clean(Some("report.PDF")),
        Ok(Some("report.PDF".to_string()))
    );
    assert!(file_field.clean(Some("malware.exe")).is_err());
    assert!(file_field.validate_bytes("empty.txt", b"").is_err());
    assert_eq!(
        file_field.validate_bytes("notes.txt", b"hello"),
        Ok("notes.txt".to_string())
    );

    let img_field = ImageField::new();
    assert_eq!(
        img_field.clean(Some("avatar.png")),
        Ok(Some("avatar.png".to_string()))
    );
    assert!(img_field.clean(Some("doc.pdf")).is_err());
    assert_eq!(
        img_field.validate_bytes("avatar.png", b"\x89PNG\r\n\x1a\nrest"),
        Ok("avatar.png".to_string())
    );
    assert_eq!(
        img_field.validate_bytes("photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
        Ok("photo.jpg".to_string())
    );
    assert_eq!(
        img_field.validate_bytes(
            "icon.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
        ),
        Ok("icon.svg".to_string())
    );
    assert!(img_field
        .validate_bytes("fake.png", b"not actually a png")
        .is_err());
}

#[test]
fn test_choice_json_ip_and_model_choice_fields() {
    let multi = MultipleChoiceField::new(vec![
        ("r".to_string(), "Red".to_string()),
        ("g".to_string(), "Green".to_string()),
        ("b".to_string(), "Blue".to_string()),
    ]);
    assert_eq!(
        multi.clean(Some("r, b")),
        Ok(vec!["r".to_string(), "b".to_string()])
    );
    assert!(multi.clean(Some("r, yellow")).is_err());
    assert!(multi.clean(None).is_err());

    let typed = TypedChoiceField::new(
        vec![
            ("1".to_string(), "Low".to_string()),
            ("2".to_string(), "High".to_string()),
        ],
        |s| s.parse::<u32>().map_err(|e| e.to_string()),
    );
    assert_eq!(typed.clean(Some("2")), Ok(Some(2u32)));
    assert!(typed.clean(Some("3")).is_err());

    let json = JsonField::new();
    assert_eq!(
        json.clean(Some(r#"{"active": true, "count": 3}"#)),
        Ok(Some(serde_json::json!({"active": true, "count": 3})))
    );
    assert!(json.clean(Some("{bad json")).is_err());

    let ip_both = GenericIpAddressField {
        unpack_ipv4: true,
        ..GenericIpAddressField::new()
    };
    assert_eq!(
        ip_both.clean(Some("192.168.1.1")),
        Ok(Some("192.168.1.1".to_string()))
    );
    assert_eq!(
        ip_both.clean(Some("::ffff:192.0.2.1")),
        Ok(Some("192.0.2.1".to_string()))
    );
    assert!(ip_both.clean(Some("999.999.999.999")).is_err());

    let ipv4_only = GenericIpAddressField {
        protocol: IpProtocol::Ipv4,
        ..GenericIpAddressField::new()
    };
    assert!(ipv4_only.clean(Some("::1")).is_err());

    let ipv6_only = GenericIpAddressField {
        protocol: IpProtocol::Ipv6,
        ..GenericIpAddressField::new()
    };
    assert_eq!(ipv6_only.clean(Some("::1")), Ok(Some("::1".to_string())));
    assert!(ipv6_only.clean(Some("127.0.0.1")).is_err());

    let model_choice = ModelChoiceField::new(vec![
        ("1".to_string(), "Alice".to_string()),
        ("2".to_string(), "Bob".to_string()),
    ]);
    assert_eq!(model_choice.clean(Some("1")), Ok(Some("1".to_string())));
    assert!(model_choice.clean(Some("99")).is_err());

    let model_multi = ModelMultipleChoiceField::new(vec![
        ("10".to_string(), "Tag A".to_string()),
        ("20".to_string(), "Tag B".to_string()),
    ]);
    assert_eq!(
        model_multi.clean(Some("10,20")),
        Ok(vec!["10".to_string(), "20".to_string()])
    );
    assert!(model_multi.clean(Some("10,30")).is_err());
}

// --- NEW TESTS: Additional Widgets & FormMedia ---

#[test]
fn test_additional_widgets_and_xss() {
    let attrs = WidgetAttrs::new();

    let file_html = FileInput.render("avatar", Some("ignored.png"), &attrs);
    assert_eq!(
        file_html,
        r#"<input type="file" name="avatar" id="id_avatar">"#
    );

    let clearable = ClearableFileInput::new();
    let clearable_html = clearable.render("avatar", Some("uploads/me.png"), &attrs);
    assert!(clearable_html.contains("avatar-clear"));
    assert!(clearable_html.contains("uploads&#x2F;me.png"));
    assert!(clearable_html.contains(r#"<input type="file" name="avatar" id="id_avatar">"#));
    assert_eq!(
        clearable.render("avatar", None, &attrs),
        r#"<input type="file" name="avatar" id="id_avatar">"#
    );

    let choices = vec![
        ("a".to_string(), "Alpha".to_string()),
        ("b".to_string(), "Beta".to_string()),
        ("c".to_string(), "Gamma".to_string()),
    ];
    let sel_multi = SelectMultiple::new(choices.clone()).render("tags", Some("a, c"), &attrs);
    assert!(sel_multi.contains(r#"<select name="tags" multiple id="id_tags">"#));
    assert!(sel_multi.contains(r#"<option value="a" selected>Alpha</option>"#));
    assert!(sel_multi.contains(r#"<option value="b">Beta</option>"#));
    assert!(sel_multi.contains(r#"<option value="c" selected>Gamma</option>"#));

    let cb_multi = CheckboxSelectMultiple::new(choices).render("tags", Some("b"), &attrs);
    assert!(cb_multi.contains(r#"id="id_tags_1" checked"#));
    assert!(!cb_multi.contains(r#"id="id_tags_0" checked"#));

    assert_eq!(
        DateTimeInput.render("starts_at", Some("2026-07-31T10:30"), &attrs),
        r#"<input type="datetime-local" name="starts_at" value="2026-07-31T10:30" id="id_starts_at">"#
    );
    assert_eq!(
        TimeInput.render("alarm", Some("07:00"), &attrs),
        r#"<input type="time" name="alarm" value="07:00" id="id_alarm">"#
    );
    assert_eq!(
        UrlInput.render("homepage", Some("https://example.com"), &attrs),
        r#"<input type="url" name="homepage" value="https:&#x2F;&#x2F;example.com" id="id_homepage">"#
    );

    let split_html = SplitDateTimeWidget.render("event_at", Some("2026-07-31 15:45"), &attrs);
    assert!(split_html.contains(
        r#"<input type="date" name="event_at_0" value="2026-07-31" id="id_event_at_0">"#
    ));
    assert!(split_html
        .contains(r#"<input type="time" name="event_at_1" value="15:45" id="id_event_at_1">"#));

    // XSS safety on new widgets
    let payload = "\"><script>alert(1)</script>";
    assert!(!clearable
        .render("f", Some(payload), &attrs)
        .contains("<script>"));
    assert!(!SplitDateTimeWidget
        .render(payload, Some(payload), &attrs)
        .contains("<script>"));
}

#[test]
fn test_form_media_merge_and_render() {
    let m1 = FormMedia::new()
        .with_css("all", "/static/base.css")
        .with_css("screen", "/static/screen.css")
        .with_js("/static/jquery.js")
        .with_js("/static/widget1.js");

    let m2 = FormMedia::new()
        .with_css("all", "/static/base.css") // duplicate
        .with_css("all", "/static/extra.css")
        .with_js("/static/jquery.js") // duplicate
        .with_js("/static/widget2.js");

    let merged = m1.merge(&m2);
    assert_eq!(
        merged.css.get("all"),
        Some(&vec![
            "/static/base.css".to_string(),
            "/static/extra.css".to_string()
        ])
    );
    assert_eq!(
        merged.js,
        vec![
            "/static/jquery.js".to_string(),
            "/static/widget1.js".to_string(),
            "/static/widget2.js".to_string()
        ]
    );

    let html = merged.render_html();
    assert!(html.contains(r#"rel="stylesheet""#));
    assert!(html.contains(r#"media="all""#));
    assert!(html.contains(r#"<script src="&#x2F;static&#x2F;widget2.js"></script>"#));
}

// --- NEW TESTS: ManagementForm, FormSet::validate, InlineFormSet ---

#[test]
fn test_management_form_parse_and_render() {
    let mgmt = ManagementForm::new("items", 3, 1, 1, 10);
    let html = mgmt.render_html();
    assert!(html.contains(r#"name="items-TOTAL_FORMS" value="3""#));
    assert!(html.contains(r#"name="items-INITIAL_FORMS" value="1""#));
    assert!(html.contains(r#"name="items-MIN_NUM_FORMS" value="1""#));
    assert!(html.contains(r#"name="items-MAX_NUM_FORMS" value="10""#));

    let mut data = HashMap::new();
    data.insert("items-TOTAL_FORMS".to_string(), "3".to_string());
    data.insert("items-INITIAL_FORMS".to_string(), "1".to_string());
    data.insert("items-MIN_NUM_FORMS".to_string(), "1".to_string());
    data.insert("items-MAX_NUM_FORMS".to_string(), "10".to_string());
    assert_eq!(ManagementForm::parse(&data, "items"), Ok(mgmt));

    let empty = HashMap::new();
    let errs = ManagementForm::parse(&empty, "items").unwrap_err();
    assert_eq!(errs.len(), 2);
}

#[test]
fn test_formset_validate_ordering_deletion_and_bounds() {
    let formset: FormSet<()> = FormSet::new()
        .with_prefix("row")
        .with_can_delete(true)
        .with_can_order(true)
        .with_min_num(2)
        .with_max_num(3)
        .with_validate_min(true)
        .with_validate_max(true);

    let mut post = HashMap::new();
    post.insert("row-TOTAL_FORMS".to_string(), "3".to_string());
    post.insert("row-INITIAL_FORMS".to_string(), "2".to_string());
    post.insert("row-0-title".to_string(), "Second".to_string());
    post.insert("row-0-ORDER".to_string(), "2".to_string());
    post.insert("row-1-title".to_string(), "To Delete".to_string());
    post.insert("row-1-DELETE".to_string(), "on".to_string());
    post.insert("row-2-title".to_string(), "First".to_string());
    post.insert("row-2-ORDER".to_string(), "1".to_string());

    let res = formset.validate(&post, |row| {
        let title = row.get("title").cloned().unwrap_or_default();
        if title.is_empty() {
            let mut errs = FormErrors::new();
            errs.add_field_error("title", "Required");
            Err(errs)
        } else {
            Ok(title)
        }
    });

    assert!(res.is_valid());
    assert_eq!(res.deleted_indices, vec![1]);
    assert_eq!(res.ordered_indices, vec![2, 0]);
    assert_eq!(
        res.cleaned_forms,
        vec!["Second".to_string(), "First".to_string()]
    );

    // Violate min_num by deleting another row (leaving only 1 active form < min_num 2)
    post.insert("row-0-DELETE".to_string(), "on".to_string());
    let under_min = formset.validate(&post, |row| {
        Ok(row.get("title").cloned().unwrap_or_default())
    });
    assert!(!under_min.is_valid());
    assert!(under_min
        .non_form_errors
        .iter()
        .any(|e| e.contains("at least 2")));
}

#[test]
fn test_inline_formset_parent_fk_injection() {
    let inline: InlineFormSet<()> = InlineFormSet::new("order_id", "42")
        .with_prefix("line")
        .with_can_delete(true);

    let mut post = HashMap::new();
    post.insert("line-TOTAL_FORMS".to_string(), "2".to_string());
    post.insert("line-INITIAL_FORMS".to_string(), "0".to_string());
    post.insert("line-0-sku".to_string(), "WIDGET-A".to_string());
    post.insert("line-1-sku".to_string(), "WIDGET-B".to_string());

    let res = inline.validate(&post, |row| {
        let fk = row.get("order_id").cloned().unwrap_or_default();
        let sku = row.get("sku").cloned().unwrap_or_default();
        Ok((fk, sku))
    });

    assert!(res.is_valid());
    assert_eq!(
        res.cleaned_forms,
        vec![
            ("42".to_string(), "WIDGET-A".to_string()),
            ("42".to_string(), "WIDGET-B".to_string()),
        ]
    );
}

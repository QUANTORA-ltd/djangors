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

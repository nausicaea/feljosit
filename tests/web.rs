//! End-to-end tests of the web application, including the security controls.

mod common;

use common::{FilePart, PASSWORD, TestApp, csrf_from, fixture, zip_files};

#[test]
fn pages_need_login() {
    let test = TestApp::new();
    test.create_user("anna");
    let mut client = test.client();
    for path in ["/budget", "/accounts", "/transactions", "/import", "/categories", "/reports"] {
        let response = client.get_no_follow(path);
        assert_eq!(response.status, 303, "{path}");
        assert_eq!(response.location(), "/login");
    }
}

#[test]
fn login_wrong_password_and_lockout() {
    let test = TestApp::new();
    test.create_user("anna");
    let mut client = test.client();
    for _ in 0..5 {
        let page = client.get("/login");
        let response = client.post(
            "/login",
            &[("username", "anna"), ("password", "wrong"), ("csrf_token", &csrf_from(&page.text()))],
        );
        assert_eq!(response.status, 401);
    }
    let page = client.get("/login");
    let response = client.post(
        "/login",
        &[("username", "anna"), ("password", PASSWORD), ("csrf_token", &csrf_from(&page.text()))],
    );
    assert_eq!(response.status, 401); // Locked.
}

#[test]
fn login_needs_csrf_cookie() {
    let test = TestApp::new();
    test.create_user("anna");
    let mut client = test.client();
    let response =
        client.post("/login", &[("username", "anna"), ("password", PASSWORD), ("csrf_token", "forged")]);
    assert_eq!(response.status, 400);
}

#[test]
fn session_cookie_flags() {
    let test = TestApp::new();
    test.create_user("anna");
    let mut client = test.client();
    let page = client.get("/login");
    let response = client.post_no_follow(
        "/login",
        &[("username", "anna"), ("password", PASSWORD), ("csrf_token", &csrf_from(&page.text()))],
    );
    let cookie = response.header("set-cookie").unwrap().to_lowercase();
    assert!(cookie.starts_with("feljosit_session="));
    assert!(cookie.contains("httponly"));
    assert!(cookie.contains("samesite=strict"));
    assert!(!cookie.contains("secure"));
}

#[test]
fn secure_cookies_and_hsts_behind_a_proxy() {
    let test = TestApp::with_settings(|s| s.secure_cookies = true);
    test.create_user("anna");
    let mut client = test.client();
    let page = client.get("/login");
    assert_eq!(page.header("strict-transport-security"), Some("max-age=31536000"));
    let response = client.post_no_follow(
        "/login",
        &[("username", "anna"), ("password", PASSWORD), ("csrf_token", &csrf_from(&page.text()))],
    );
    assert!(response.header("set-cookie").unwrap().contains("; Secure"));
}

#[test]
fn security_headers() {
    let test = TestApp::new();
    let (mut http, _) = test.anna();
    let response = http.get("/budget");
    let csp = response.header("content-security-policy").unwrap();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("script-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    // "no-referrer" makes browsers send "Origin: null". This breaks the Origin check.
    assert_eq!(response.header("referrer-policy"), Some("same-origin"));
}

#[test]
fn static_files() {
    let test = TestApp::new();
    let mut client = test.client();
    let response = client.get("/static/app.css");
    assert_eq!(response.status, 200);
    assert!(response.header("content-type").unwrap().starts_with("text/css"));
    assert_eq!(response.header("cache-control"), None);
    assert_eq!(client.get("/static/app.js").status, 200);
    assert_eq!(client.get("/static/icon.svg").status, 200);
    assert_eq!(client.get("/static/../Cargo.toml").status, 404);
    assert_eq!(client.get("/healthz").text(), "ok");
}

#[test]
fn no_api_docs() {
    let test = TestApp::new();
    let mut client = test.client();
    for path in ["/docs", "/redoc", "/openapi.json"] {
        assert_eq!(client.get(path).status, 404);
    }
}

#[test]
fn unknown_host_refused() {
    // DNS rebinding protection.
    let test = TestApp::new();
    let mut client = test.client();
    client.host = "attacker.example".into();
    assert_eq!(client.get("/login").status, 400);
    client.host = "127.0.0.1:8765".into();
    assert_eq!(client.get_no_follow("/login").status, 303); // No user: the setup page.
    client.host = "[::1]:8765".into();
    assert_eq!(client.get_no_follow("/login").status, 303);
}

#[test]
fn post_without_csrf_token_refused() {
    let test = TestApp::new();
    let (mut http, _) = test.anna();
    let response = http.post("/accounts", &[("name", "X"), ("account_type", "cash")]);
    assert_eq!(response.status, 403);
    let response =
        http.post("/accounts", &[("name", "X"), ("account_type", "cash"), ("csrf_token", "wrong")]);
    assert_eq!(response.status, 403);
}

#[test]
fn cross_origin_post_refused() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let form = [("name", "X"), ("account_type", "cash"), ("csrf_token", token.as_str())];
    let response = http.post_with_headers("/accounts", &form, &[("Origin", "http://evil.example")]);
    assert_eq!(response.status, 403);
    let response = http.post_with_headers("/accounts", &form, &[("Origin", "null")]);
    assert_eq!(response.status, 403);
    let response = http.post_with_headers("/accounts", &form, &[("Origin", "http://127.0.0.1")]);
    assert_eq!(response.status, 303);
}

#[test]
fn logout_ends_session() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.post("/logout", &[("csrf_token", &token)]);
    assert_eq!(http.get_no_follow("/budget").status, 303);
}

#[test]
fn body_size_limit() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let big = vec![b'x'; 11 * 1024 * 1024];
    let response = http.post_files(
        "/import",
        &[("csrf_token", &token)],
        &[FilePart { field: "statement", filename: "big.xml", content_type: "application/xml", data: &big }],
    );
    assert_eq!(response.status, 413);
}

#[test]
fn full_flow_import_categorise_budget_reports() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("iban", "CH89 0023 5000 0123 4567 8")]);
    let text = http.import_fixture(&token, "camt053_v04_private.xml", "");
    assert!(text.contains("7 new transactions"), "{text}");
    assert!(text.contains("agrees with the account"));
    // Second import of the same file.
    assert!(http.import_fixture(&token, "camt053_v04_private.xml", "").contains("imported before"));

    let tx: i64 = test.scalar("SELECT id FROM transactions WHERE counterparty = 'Muster AG'");
    let salary: i64 = test.scalar("SELECT id FROM categories WHERE name = 'Salary'");
    let groceries: i64 = test.scalar("SELECT id FROM categories WHERE name = 'Groceries'");
    let response = http.post(
        &format!("/transactions/{tx}/category"),
        &[("csrf_token", &token), ("category_id", &salary.to_string()), ("return_to", "/transactions")],
    );
    assert_eq!(response.status, 200);
    let response = http.post(
        "/budget/assign",
        &[
            ("csrf_token", &token),
            ("category_id", &groceries.to_string()),
            ("month", "2026-10"),
            ("amount", "1'000.00"),
        ],
    );
    assert_eq!(response.status, 200);
    let page = http.get("/budget?month=2026-10").text();
    // 1000.00 starting balance + 6500.00 salary - 1000.00 assigned.
    assert!(page.contains("CHF 6&#39;500.00"), "{page}");
    assert_eq!(http.get("/reports?period=24m").status, 200);
    let response = http.get("/reports?from=2026-10-01&to=2026-10-31");
    assert_eq!(response.status, 200);
    assert!(response.text().contains("01.10.2026 to 31.10.2026"));
    assert!(response.text().contains("Remove the custom period"));
    let response = http.get("/reports?from=2026-11-01&to=2026-10-01");
    assert_eq!(response.status, 400);
    assert!(response.text().contains("on or before the end date"));
    assert_eq!(http.get("/reports?from=&to=").status, 200);
    let response = http.get("/reports?period=all");
    assert_eq!(response.status, 200);
    assert!(response.text().contains(r#"href="/reports?period=all" aria-current="page""#));
    assert_eq!(http.get("/reports?from=abc").status, 422);
    assert!(http.get("/transactions?q=muster").text().contains("Muster AG"));
    for path in [
        "/reports/categories?period=all",
        "/reports/counterparties?period=all",
        "/reports/categories?period=all&sort=total",
        "/reports/categories?period=all&sort=change",
        "/reports/counterparties?period=all&counterparty=Muster+AG",
        "/accounts",
        "/categories",
        "/settings",
        "/import",
        "/transactions?sort=size_desc&iban=true",
    ] {
        assert_eq!(http.get(path).status, 200, "{path}");
    }
}

#[test]
fn import_with_two_accounts_is_all_or_nothing() {
    // If one statement of a file has an error, no statement of the file is imported.
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("iban", "CH89 0023 5000 0123 4567 8")]);
    http.add_account(
        &token,
        &[("name", "Savings"), ("account_type", "savings"), ("iban", "CH03 0070 0110 0001 2345 6")],
    );
    let data = zip_files(&[
        ("private.xml", fixture("camt053_v04_private.xml")),
        ("savings.xml", fixture("camt053_v08_savings.xml")),
    ]);
    let private: i64 = test.scalar("SELECT id FROM accounts WHERE name = 'Private'");
    // The private statement is valid for the selected account. The savings statement is not.
    let text = http.import(&token, "both.zip", &data, &private.to_string()).text();
    assert!(text.contains("belongs to a different account"));
    assert_eq!(test.scalar::<i64>("SELECT count(*) FROM transactions"), 0);
    assert_eq!(test.scalar::<i64>("SELECT count(*) FROM import_batches"), 0);
    // Without a selected account, the same file goes to the two accounts.
    let text = http.import(&token, "both.zip", &data, "").text();
    assert!(text.contains("7 new transactions"));
    assert!(text.contains("2 new transactions"));
    assert!(!text.contains("belongs to a different account"));
}

#[test]
fn transaction_filter_form_with_empty_account() {
    // The filter form sends `account=` for "All accounts"; this must not give 422.
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.import_fixture(&token, "camt053_v04_private.xml", "");
    assert_eq!(http.get("/transactions?account=&q=&uncategorised=true").status, 200);
    assert_eq!(http.get("/transactions?account=&category=").status, 200);
    assert_eq!(http.get("/transactions?account=abc").status, 422);
}

#[test]
fn import_errors_are_shown() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let response = http.import(&token, "x.xml", b"<not-camt/>", "");
    assert_eq!(response.status, 400);
    assert!(response.text().contains("no ISO 20022 namespace"));
    let response = http.import(&token, "x.xml", br#"<D xmlns="urn:x"/>"#, "");
    assert_eq!(response.status, 400);
    assert!(response.text().contains("not a camt.053"));
    let response = http.import(&token, "x.xml", b"", "");
    assert!(response.text().contains("The file is empty."));
}

#[test]
fn html_is_escaped() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("name", "<script>alert(1)</script>")]);
    let page = http.get("/accounts").text();
    assert!(!page.contains("<script>alert(1)</script>"));
    assert!(page.contains("&lt;script&gt;"));
}

#[test]
fn users_cannot_see_or_change_data_of_other_users() {
    let test = TestApp::new();
    test.create_user("anna");
    test.create_user("bob");
    let (mut anna, anna_token) = test.login("anna");
    let (mut bob, bob_token) = test.login("bob");
    anna.add_account(&anna_token, &[("name", "Anna secret account"), ("iban", "CH8900235000012345678")]);
    anna.import_fixture(&anna_token, "camt053_v04_private.xml", "");
    let anna_id: i64 = test.scalar("SELECT id FROM users WHERE username = 'anna'");
    let account: i64 = test.scalar(&format!("SELECT id FROM accounts WHERE user_id = {anna_id}"));
    let tx: i64 = test.scalar(&format!("SELECT id FROM transactions WHERE user_id = {anna_id}"));
    let category: i64 = test.scalar(&format!("SELECT id FROM categories WHERE user_id = {anna_id}"));

    assert!(!bob.get("/accounts").text().contains("Anna secret account"));
    assert!(!bob.get(&format!("/transactions?account={account}")).text().contains("Muster AG"));
    let category_text = category.to_string();
    let cases: Vec<(String, Vec<(&str, &str)>)> = vec![
        (format!("/accounts/{account}/close"), vec![]),
        (format!("/accounts/{account}/edit"), vec![]),
        (format!("/transactions/{tx}/category"), vec![("category_id", "")]),
        (format!("/transactions/{tx}/memo"), vec![("memo", "hacked")]),
        (format!("/categories/{category}/rename"), vec![("name", "hacked")]),
        (
            "/budget/assign".into(),
            vec![("category_id", &category_text), ("month", "2026-10"), ("amount", "1")],
        ),
    ];
    for (path, data) in cases {
        let mut form = vec![("csrf_token", bob_token.as_str())];
        form.extend(data);
        let response = if path.ends_with("/edit") { bob.get(&path) } else { bob.post(&path, &form) };
        assert_eq!(response.status, 404, "{path}");
    }
    // Bob cannot import into the account of Anna.
    let response = bob.import(&bob_token, "s.xml", &fixture("camt053_v08_savings.xml"), &account.to_string());
    assert_eq!(response.status, 404);
}

#[test]
fn open_redirect_prevented() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("name", "Cash"), ("account_type", "cash")]);
    let response = http.post(
        "/transactions",
        &[
            ("csrf_token", &token),
            ("account_id", "1"),
            ("booking_date", "2026-01-01"),
            ("amount", "5"),
            ("direction", "outflow"),
        ],
    );
    assert_eq!(response.status, 200);
    let response = http.post_no_follow(
        "/transactions/1/memo",
        &[("csrf_token", &token), ("memo", "x"), ("return_to", "//evil.example/")],
    );
    assert_eq!(response.location(), "/transactions");
    for (target, expected) in [
        ("https://evil.example/transactions?q=a", "/transactions?q=a"),
        ("/transactions?account=1&next=//evil&page=2", "/transactions?account=1&page=2"),
        ("/\\evil.example", "/transactions"),
    ] {
        let response = http.post_no_follow(
            "/transactions/1/memo",
            &[("csrf_token", &token), ("memo", "x"), ("return_to", target)],
        );
        assert_eq!(response.location(), expected, "{target}");
    }
}

#[test]
fn change_password_closes_other_sessions() {
    let test = TestApp::new();
    test.create_user("anna");
    let (mut first, first_token) = test.login("anna");
    let (mut second, _) = test.login("anna");
    let response = first.post(
        "/settings/password",
        &[
            ("csrf_token", &first_token),
            ("current_password", PASSWORD),
            ("new_password", "a new long password"),
            ("new_password_repeat", "a new long password"),
        ],
    );
    assert_eq!(response.status, 200);
    assert!(response.text().contains("password is changed"));
    assert_eq!(second.get_no_follow("/budget").status, 303);
    assert_eq!(first.get_no_follow("/budget").status, 200);
}

#[test]
fn validation_error_does_not_echo_input() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let response =
        http.post("/accounts", &[("csrf_token", &token), ("name", "x"), ("account_type", "<b>evil</b>")]);
    assert_eq!(response.status, 422);
    assert!(!response.text().contains("evil"));
}

#[test]
fn every_form_has_a_csrf_token() {
    // Each POST form must contain a non-empty CSRF token (regression test).
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("iban", "CH8900235000012345678")]);
    http.import_fixture(&token, "camt053_v04_private.xml", "");
    for path in ["/budget", "/accounts", "/transactions", "/import", "/categories", "/settings"] {
        let html = http.get(path).text();
        let forms: Vec<&str> = html
            .split(r#"<form method="post""#)
            .skip(1)
            .map(|f| f.split("</form>").next().unwrap())
            .collect();
        assert!(!forms.is_empty(), "{path}");
        for form in forms {
            assert!(
                form.contains(&format!(r#"name="csrf_token" value="{token}""#)),
                "{path}: {}",
                &form[..form.len().min(200)]
            );
        }
    }
}

#[test]
fn assign_redirect_uses_only_the_parsed_month() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let groceries = test.scalar::<i64>("SELECT id FROM categories WHERE name = 'Groceries'").to_string();
    let data = |month: &'static str| {
        vec![
            ("csrf_token", token.clone()),
            ("category_id", groceries.clone()),
            ("amount", "x".into()),
            ("month", month.into()),
        ]
    };
    let post = |http: &mut common::Client, month: &'static str| {
        let form = data(month);
        let refs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        http.post_no_follow("/budget/assign", &refs)
    };
    assert_eq!(post(&mut http, "/evil").status, 422);
    let response = post(&mut http, "2026-13");
    assert_eq!(response.status, 303);
    assert!(response.location().starts_with("/budget?month="));
    assert!(!response.location().contains("2026-13"));
    assert_eq!(post(&mut http, "2026-10").location(), "/budget?month=2026-10");
}

#[test]
fn malformed_statement_values_give_an_error_page_not_500() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("iban", "CH89 0023 5000 0123 4567 8")]);
    let original = String::from_utf8(fixture("camt053_v04_private.xml")).unwrap();
    for (old, new) in
        [("<BookgDt><Dt>2026-10-", "<BookgDt><Dt>2026-13-"), (r#"Ccy="CHF">"#, r#"Ccy="CHF">1E999999"#)]
    {
        assert!(original.contains(old));
        let response = http.import(&token, "s.xml", original.replacen(old, new, 1).as_bytes(), "");
        assert_eq!(response.status, 400, "{old}");
        assert!(!response.text().contains("1E999999"));
    }
}

#[test]
fn first_user_in_the_browser() {
    let test = TestApp::new();
    let mut client = test.client();
    let response = client.get_no_follow("/login");
    assert_eq!(response.location(), "/setup");
    let page = client.get("/setup");
    assert_eq!(page.status, 200);
    let token = csrf_from(&page.text());
    // The two passwords must be the same.
    let response = client.post(
        "/setup",
        &[
            ("csrf_token", &token),
            ("username", "anna"),
            ("password", PASSWORD),
            ("password_repeat", "different password"),
        ],
    );
    assert_eq!(response.status, 400);
    assert!(response.text().contains("not the same"));
    let token = csrf_from(&response.text());
    let response = client.post(
        "/setup",
        &[
            ("csrf_token", &token),
            ("username", "anna"),
            ("password", PASSWORD),
            ("password_repeat", PASSWORD),
        ],
    );
    assert_eq!(response.status, 200);
    assert!(response.text().contains("The user &#39;anna&#39; is added."), "{}", response.text());
    // Now the setup page is closed.
    assert_eq!(client.get_no_follow("/setup").location(), "/login");
    let mut other = test.client();
    assert_eq!(other.get_no_follow("/login").status, 200);
}

#[test]
fn setup_needs_the_cookie_and_the_local_computer() {
    let test = TestApp::new();
    let mut client = test.client();
    let response = client.post(
        "/setup",
        &[
            ("csrf_token", "forged"),
            ("username", "anna"),
            ("password", PASSWORD),
            ("password_repeat", PASSWORD),
        ],
    );
    assert_eq!(response.status, 400);
    assert_eq!(test.scalar::<i64>("SELECT count(*) FROM users"), 0);
    let mut remote = test.client();
    remote.loopback = false;
    assert_eq!(remote.get("/setup").status, 403);
    // A reverse proxy on the same computer sends the public host name.
    let proxy = TestApp::with_settings(|s| s.allowed_hosts.push("budget.example".into()));
    let mut client = proxy.client();
    client.host = "budget.example".into();
    assert_eq!(client.get("/setup").status, 403);
}

#[test]
fn head_and_wrong_methods() {
    let test = TestApp::new();
    let mut client = test.client();
    let response = client.send("HEAD", "/healthz", &[], Vec::new());
    assert_eq!(response.status, 200);
    assert!(response.body.is_empty());
    assert_eq!(client.send("DELETE", "/budget", &[], Vec::new()).status, 405);
}

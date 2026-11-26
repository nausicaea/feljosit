//! Categories, groups and categorisation rules.

mod common;

use common::*;
use feljosit::models::{CategorisationRule, Category, CategoryGroup, MatchType, RuleField, User};
use feljosit::services::categorise::{self, RuleValues};
use feljosit::services::{Error as ServiceError, categories, users};
use rusqlite::Connection;

fn groups(conn: &Connection, user: &User) -> std::collections::HashMap<String, CategoryGroup> {
    CategoryGroup::list(conn, user.id).unwrap().into_iter().map(|g| (g.name.clone(), g)).collect()
}

fn names(conn: &Connection, group_id: i64) -> Vec<String> {
    let group = CategoryGroup::get(conn, group_id).unwrap().unwrap();
    CategoryGroup::list(conn, group.user_id)
        .unwrap()
        .into_iter()
        .find(|g| g.id == group_id)
        .unwrap()
        .categories
        .into_iter()
        .map(|c| c.name)
        .collect()
}

fn sort_orders(conn: &Connection, group_id: i64) -> Vec<i64> {
    let mut stmt =
        conn.prepare("SELECT sort_order FROM categories WHERE group_id = ?1 ORDER BY sort_order").unwrap();
    stmt.query_map([group_id], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
}

fn category(conn: &Connection, user: &User, name: &str) -> Category {
    Category::get(conn, category_id(conn, user, name)).unwrap().unwrap()
}

#[test]
fn move_into_other_group() {
    let (conn, user) = database_with_user();
    let all = groups(&conn, &user);
    let (fixed, daily) = (all["Fixed costs"].id, all["Daily life"].id);
    let before_fixed = names(&conn, fixed);
    let before_daily = names(&conn, daily);
    let fixed_group = CategoryGroup::get(&conn, fixed).unwrap().unwrap();
    // To the end of the group.
    categories::move_category(&conn, &category(&conn, &user, "Groceries"), &fixed_group, None).unwrap();
    let mut expected = before_fixed.clone();
    expected.push("Groceries".into());
    assert_eq!(names(&conn, fixed), expected);
    assert_eq!(
        names(&conn, daily),
        before_daily.iter().filter(|n| *n != "Groceries").cloned().collect::<Vec<_>>()
    );
    // The sort orders have no gaps.
    assert_eq!(sort_orders(&conn, fixed), (0..expected.len() as i64).collect::<Vec<_>>());
    assert_eq!(sort_orders(&conn, daily), (0..before_daily.len() as i64 - 1).collect::<Vec<_>>());
    // Back, before the category that is the first of the group now.
    let first = names(&conn, daily)[0].clone();
    let daily_group = CategoryGroup::get(&conn, daily).unwrap().unwrap();
    let before = category(&conn, &user, &first);
    categories::move_category(&conn, &category(&conn, &user, "Groceries"), &daily_group, Some(&before))
        .unwrap();
    assert_eq!(names(&conn, daily)[..2], ["Groceries".to_string(), first]);
    assert_eq!(names(&conn, fixed), before_fixed);
}

#[test]
fn move_inside_the_group() {
    let (conn, user) = database_with_user();
    let fixed = CategoryGroup::get(&conn, groups(&conn, &user)["Fixed costs"].id).unwrap().unwrap();
    let list = names(&conn, fixed.id);
    let last = category(&conn, &user, list.last().unwrap());
    let first = category(&conn, &user, &list[0]);
    categories::move_category(&conn, &last, &fixed, Some(&first)).unwrap();
    let mut expected = vec![list.last().unwrap().clone()];
    expected.extend_from_slice(&list[..list.len() - 1]);
    assert_eq!(names(&conn, fixed.id), expected);
    // Before itself: no change.
    let last = category(&conn, &user, list.last().unwrap());
    categories::move_category(&conn, &last, &fixed, Some(&last)).unwrap();
    assert_eq!(names(&conn, fixed.id), expected);
}

#[test]
fn move_into_empty_group() {
    let (conn, user) = database_with_user();
    conn.execute(
        "INSERT INTO category_groups (user_id, name, sort_order) VALUES (?1, 'Empty', 99)",
        [user.id],
    )
    .unwrap();
    let empty = CategoryGroup::get(&conn, conn.last_insert_rowid()).unwrap().unwrap();
    categories::move_category(&conn, &category(&conn, &user, "Groceries"), &empty, None).unwrap();
    assert_eq!(names(&conn, empty.id), vec!["Groceries"]);
    assert_eq!(category(&conn, &user, "Groceries").sort_order, 0);
}

#[test]
fn move_errors() {
    let (conn, user) = database_with_user();
    let all = groups(&conn, &user);
    let groceries = category(&conn, &user, "Groceries");
    let rent = category(&conn, &user, "Rent and service charges");
    let daily = CategoryGroup::get(&conn, all["Daily life"].id).unwrap().unwrap();
    // The position must be in the target group.
    assert_ne!(rent.group_id, daily.id);
    let error = categories::move_category(&conn, &groceries, &daily, Some(&rent)).unwrap_err();
    assert!(error.to_string().contains("target group"));
    let other = users::create_user(&conn, "bob", "another long password").unwrap();
    let other_group = CategoryGroup::list(&conn, other.id).unwrap().remove(0);
    assert!(matches!(
        categories::move_category(&conn, &groceries, &other_group, None),
        Err(ServiceError::NotFound(_))
    ));
    assert!(matches!(
        categories::owned_group(&conn, user.id, other_group.id),
        Err(ServiceError::NotFound(_))
    ));
}

fn rule(conn: &Connection, user: &User) -> i64 {
    let values = RuleValues {
        field: RuleField::Counterparty,
        match_type: MatchType::Contains,
        pattern: "Migros",
        category_id: category_id(conn, user, "Groceries"),
        priority: 100,
    };
    categorise::create_rule(conn, user.id, &values).unwrap()
}

#[test]
fn update_rule() {
    let (conn, user) = database_with_user();
    let id = rule(&conn, &user);
    let eating_out = category_id(&conn, &user, "Eating out");
    let stored = categorise::owned_rule(&conn, user.id, id).unwrap();
    let values = RuleValues {
        field: RuleField::Description,
        match_type: MatchType::Equals,
        pattern: "  Migros Restaurant  ",
        category_id: eating_out,
        priority: 5,
    };
    categorise::update_rule(&conn, &stored, &values).unwrap();
    let stored: CategorisationRule = categorise::owned_rule(&conn, user.id, id).unwrap();
    assert_eq!(
        (stored.field, stored.match_type, stored.pattern.as_str()),
        (RuleField::Description, MatchType::Equals, "Migros Restaurant")
    );
    assert_eq!((stored.category_id, stored.priority), (eating_out, 5));
}

#[test]
fn update_rule_checks_values() {
    let long = "x".repeat(141);
    for (pattern, priority, message) in
        [(" ", 100, "1 to 140"), (long.as_str(), 100, "1 to 140"), ("x", 10_001, "priority")]
    {
        let (conn, user) = database_with_user();
        let stored = categorise::owned_rule(&conn, user.id, rule(&conn, &user)).unwrap();
        let values = RuleValues {
            field: stored.field,
            match_type: stored.match_type,
            pattern,
            category_id: stored.category_id,
            priority,
        };
        let error = categorise::update_rule(&conn, &stored, &values).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(categorise::owned_rule(&conn, user.id, stored.id).unwrap().pattern, "Migros");
    }
}

#[test]
fn update_rule_refuses_category_of_other_user() {
    let (conn, user) = database_with_user();
    let stored = categorise::owned_rule(&conn, user.id, rule(&conn, &user)).unwrap();
    let other = users::create_user(&conn, "bob", "another long password").unwrap();
    let values = RuleValues {
        field: stored.field,
        match_type: stored.match_type,
        pattern: "x",
        category_id: category_id(&conn, &other, "Groceries"),
        priority: 1,
    };
    assert!(matches!(categorise::update_rule(&conn, &stored, &values), Err(ServiceError::NotFound(_))));
    assert!(matches!(categorise::owned_rule(&conn, other.id, stored.id), Err(ServiceError::NotFound(_))));
}

#[test]
fn rules_compare_text_without_case() {
    let (conn, user) = database_with_user();
    let stored = categorise::owned_rule(&conn, user.id, rule(&conn, &user)).unwrap();
    let candidate =
        |counterparty| categorise::Candidate { counterparty, counterparty_iban: None, description: "" };
    assert!(categorise::rule_matches(&stored, &candidate("MIGROS Zürich")));
    assert!(!categorise::rule_matches(&stored, &candidate("Coop")));
}

#[test]
fn categories_groups_and_rules_in_the_browser() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    assert!(http.get("/categories").text().contains("Groceries"));
    http.post("/categories/groups", &[("csrf_token", &token), ("name", "Hobbies")]);
    let group: i64 = test.scalar("SELECT id FROM category_groups WHERE name = 'Hobbies'");
    http.post(
        "/categories",
        &[("csrf_token", &token), ("group_id", &group.to_string()), ("name", "Climbing")],
    );
    let climbing: i64 = test.scalar("SELECT id FROM categories WHERE name = 'Climbing'");
    http.post(&format!("/categories/{climbing}/rename"), &[("csrf_token", &token), ("name", "Bouldering")]);
    http.post(&format!("/categories/{climbing}/hide"), &[("csrf_token", &token), ("hidden", "true")]);
    assert_eq!(
        test.scalar::<String>(&format!("SELECT name FROM categories WHERE id = {climbing}")),
        "Bouldering"
    );
    assert!(test.scalar::<bool>(&format!("SELECT hidden FROM categories WHERE id = {climbing}")));

    // A name that is too short is refused with a message.
    let page = http.post("/categories/groups", &[("csrf_token", &token), ("name", "  ")]).text();
    assert!(page.contains("1 to 100 characters"));

    http.add_account(&token, &[("iban", "CH8900235000012345678")]);
    http.import_fixture(&token, "camt053_v04_private.xml", "");
    let rent: i64 = test.scalar("SELECT id FROM categories WHERE name = 'Rent and service charges'");
    let page = http
        .post(
            "/rules",
            &[
                ("csrf_token", &token),
                ("field", "counterparty"),
                ("match_type", "contains"),
                ("pattern", "Immo"),
                ("category_id", &rent.to_string()),
                ("priority", "10"),
            ],
        )
        .text();
    assert!(page.contains("The rule is added"));
    assert!(page.contains("counterparty contains"));
    let page = http.post("/rules/apply", &[("csrf_token", &token)]).text();
    assert!(page.contains("1 transactions are categorised"));
    let rule: i64 = test.scalar("SELECT id FROM categorisation_rules");
    http.post(&format!("/rules/{rule}/delete"), &[("csrf_token", &token)]);
    assert_eq!(test.scalar::<i64>("SELECT count(*) FROM categorisation_rules"), 0);
}

#[test]
fn manual_transaction_flow() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    http.add_account(&token, &[("name", "Wallet"), ("account_type", "cash"), ("starting_balance", "200")]);
    let page = http
        .post(
            "/transactions",
            &[
                ("csrf_token", &token),
                ("account_id", "1"),
                ("booking_date", "2026-03-01"),
                ("amount", "12.50"),
                ("direction", "outflow"),
                ("counterparty", "Bakery"),
            ],
        )
        .text();
    assert!(page.contains("The transaction is added"));
    assert!(page.contains("-12.50"));
    let page = http
        .post(
            "/transactions",
            &[
                ("csrf_token", &token),
                ("account_id", "1"),
                ("booking_date", "2026-03-01"),
                ("amount", "abc"),
                ("direction", "inflow"),
            ],
        )
        .text();
    assert!(page.contains("not valid"));
    let page =
        http.post("/transactions/2/delete", &[("csrf_token", &token), ("return_to", "/transactions")]).text();
    assert!(page.contains("is deleted"));
    // The starting balance is not a manual transaction.
    let page = http.post("/transactions/1/delete", &[("csrf_token", &token)]).text();
    assert!(page.contains("only delete"));
    assert!(http.get("/accounts").text().contains("200.00"));
    let page = http.post("/accounts/1/close", &[("csrf_token", &token), ("closed", "true")]).text();
    assert!(page.contains("(closed)"));
}

fn ids(test: &TestApp) -> (i64, i64, i64, i64) {
    (
        test.scalar("SELECT id FROM categories WHERE name = 'Groceries'"),
        test.scalar("SELECT id FROM categories WHERE name = 'Rent and service charges'"),
        test.scalar("SELECT id FROM category_groups WHERE name = 'Fixed costs'"),
        test.scalar("SELECT id FROM category_groups WHERE name = 'Daily life'"),
    )
}

fn group_of(test: &TestApp, category: i64) -> i64 {
    test.scalar(&format!("SELECT group_id FROM categories WHERE id = {category}"))
}

#[test]
fn move_with_the_form() {
    // Without JavaScript: the group list of the category sends a normal form.
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let (groceries, _, fixed, _) = ids(&test);
    let page = http.get("/categories").text();
    assert!(page.contains(&format!(r#"action="/categories/{groceries}/move""#)));
    assert!(page.contains(r#"data-category-id=""#) && page.contains(r#"class="drag-handle""#));
    let response = http.post(
        &format!("/categories/{groceries}/move"),
        &[("csrf_token", &token), ("group_id", &fixed.to_string())],
    );
    assert_eq!(response.status, 200);
    assert!(response.text().contains("is in the group &#39;Fixed costs&#39; now"));
    assert_eq!(group_of(&test, groceries), fixed);
}

#[test]
fn move_with_json() {
    // The drag-and-drop script asks for JSON.
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let (groceries, rent, fixed, daily) = ids(&test);
    let json = [("Accept", "application/json")];
    let target = format!("/categories/{groceries}/move");
    let response = http.post_with_headers(
        &target,
        &[("csrf_token", &token), ("group_id", &fixed.to_string()), ("before_id", &rent.to_string())],
        &json,
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.text(), r#"{"group":"Fixed costs"}"#);
    assert_eq!(group_of(&test, groceries), fixed);
    // A position in a different group.
    let response = http.post_with_headers(
        &target,
        &[("csrf_token", &token), ("group_id", &daily.to_string()), ("before_id", &rent.to_string())],
        &json,
    );
    assert_eq!(response.status, 400);
    assert!(response.text().contains("target group"));
    assert_eq!(group_of(&test, groceries), fixed);
    // Unknown IDs.
    assert_eq!(
        http.post("/categories/999999/move", &[("csrf_token", &token), ("group_id", &daily.to_string())])
            .status,
        404
    );
    assert_eq!(http.post(&target, &[("csrf_token", &token), ("group_id", "999999")]).status, 404);
    // The CSRF token is necessary.
    assert!([400, 403].contains(&http.post(&target, &[("group_id", &daily.to_string())]).status));
    assert_eq!(group_of(&test, groceries), fixed);
}

#[test]
fn edit_rule_pages() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let (rule_id, eating_out) = test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        (rule(conn, &user), category_id(conn, &user, "Eating out"))
    });
    assert!(http.get("/categories").text().contains(&format!(r#"href="/rules/{rule_id}/edit""#)));
    let page = http.get(&format!("/rules/{rule_id}/edit"));
    assert_eq!(page.status, 200);
    assert!(page.text().contains(r#"value="Migros""#));
    assert!(page.text().contains(r#"value="counterparty" selected"#));

    let eating_out = eating_out.to_string();
    let mut data = vec![
        ("csrf_token", token.as_str()),
        ("field", "description"),
        ("match_type", "equals"),
        ("pattern", "Coop"),
        ("category_id", eating_out.as_str()),
        ("priority", "7"),
    ];
    let response = http.post(&format!("/rules/{rule_id}"), &data);
    assert_eq!(response.status, 200);
    assert!(response.text().contains("The rule is changed."));
    assert!(response.text().contains("Coop"));
    let stored = test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        categorise::owned_rule(conn, user.id, rule_id).unwrap()
    });
    assert_eq!(
        (
            stored.field,
            stored.match_type,
            stored.pattern.as_str(),
            stored.category_id.to_string(),
            stored.priority
        ),
        (RuleField::Description, MatchType::Equals, "Coop", eating_out.clone(), 7)
    );
    // An error goes back to the edit page with a message.
    data[3] = ("pattern", "   ");
    let response = http.post(&format!("/rules/{rule_id}"), &data);
    assert!(response.text().contains("1 to 140 characters"));
    assert!(response.text().contains(&format!(r#"action="/rules/{rule_id}""#)));
    // "Apply" is not a rule ID.
    assert!(
        http.post("/rules/apply", &[("csrf_token", &token)]).text().contains("transactions are categorised")
    );
    // Unknown rules and the CSRF token.
    assert_eq!(http.get("/rules/999999/edit").status, 404);
    data[3] = ("pattern", "Coop");
    assert_eq!(http.post("/rules/999999", &data).status, 404);
    data[0] = ("csrf_token", "wrong");
    assert!([400, 403].contains(&http.post(&format!("/rules/{rule_id}"), &data).status));
}

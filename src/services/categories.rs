//! The structure of the categories: groups and the order of the categories.

use rusqlite::{Connection, params};

use super::{Error, Result};
use crate::models::{Category, CategoryGroup};

pub fn owned_group(conn: &Connection, user_id: i64, group_id: i64) -> Result<CategoryGroup> {
    match CategoryGroup::get(conn, group_id)? {
        Some(group) if group.user_id == user_id => Ok(group),
        _ => Err(Error::NotFound("category group")),
    }
}

fn ordered_ids(conn: &Connection, group_id: i64) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id FROM categories WHERE group_id = ?1 ORDER BY sort_order, name, id")?;
    let rows = stmt.query_map([group_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn renumber(conn: &Connection, ids: &[i64]) -> Result<()> {
    for (order, id) in ids.iter().enumerate() {
        conn.execute("UPDATE categories SET sort_order = ?1 WHERE id = ?2", params![order as i64, id])?;
    }
    Ok(())
}

/// Move `category` into `group`, before the category `before`.
///
/// If `before` is `None`, the category becomes the last category of the group. The
/// category can stay in its group: then only its position changes. The categories of
/// the old and the new group get the sort orders 0, 1, 2, … again. The transactions
/// and the budget amounts of the category do not change.
pub fn move_category(
    conn: &Connection,
    category: &Category,
    group: &CategoryGroup,
    before: Option<&Category>,
) -> Result<()> {
    if group.user_id != category.user_id {
        return Err(Error::NotFound("category group"));
    }
    if let Some(before) = before {
        if before.id == category.id {
            return Ok(());
        }
        if before.group_id != group.id {
            return Err(Error::service("The target position is not in the target group."));
        }
    }
    let old_group = (category.group_id != group.id).then_some(category.group_id);
    let mut target: Vec<i64> =
        ordered_ids(conn, group.id)?.into_iter().filter(|id| *id != category.id).collect();
    let index = match before {
        None => target.len(),
        Some(before) => target.iter().position(|id| *id == before.id).unwrap_or(target.len()),
    };
    target.insert(index, category.id);
    conn.execute("UPDATE categories SET group_id = ?1 WHERE id = ?2", params![group.id, category.id])?;
    renumber(conn, &target)?;
    if let Some(old) = old_group {
        let remaining: Vec<i64> =
            ordered_ids(conn, old)?.into_iter().filter(|id| *id != category.id).collect();
        renumber(conn, &remaining)?;
    }
    Ok(())
}

-- 0005_add_meals.sql — meals, grocery lists/items, and pantry (P2 Slice 2).
--
-- Five tables back P2 Slice 1's domain types (amity_core::meal, ::grocery,
-- ::pantry):
--   • meals             — one planned meal on one date.
--   • meal_ingredients  — a meal's freetext ingredient lines, order-preserving
--                         via `position` (Meal.ingredient_lines is a Vec, and
--                         SQL rows have no inherent order).
--   • grocery_lists     — a named grocery list.
--   • grocery_items     — one line on a list, manual or generated from a meal.
--   • pantry_items      — the household's staples memory (see amity_core::pantry).
--
-- STRICT tables have no implicit `ON DELETE CASCADE` unless declared on the
-- FK, and none is declared here (consistent with migrations 0003/0004) — the
-- repository layer deletes child rows explicitly before their parent.

CREATE TABLE meals (
    id         TEXT    NOT NULL PRIMARY KEY,  -- UUID v7
    date       TEXT    NOT NULL,              -- YYYY-MM-DD
    slot       TEXT    NOT NULL,              -- snake_case MealSlot
    name       TEXT    NOT NULL,              -- non-empty (validated by MealBuilder)
    cook_id    TEXT,                          -- MemberId, NULL if unassigned
    notes      TEXT,                          -- free-form, NULL if none
    created_at TEXT    NOT NULL               -- RFC 3339
) STRICT;

-- Speeds up the generator's "meals in [from, to]" query (list_meals_in_range).
CREATE INDEX idx_meals_date ON meals (date);

CREATE TABLE meal_ingredients (
    id       TEXT    NOT NULL PRIMARY KEY,    -- UUID v7 (not surfaced on IngredientLine)
    meal_id  TEXT    NOT NULL REFERENCES meals (id),
    position INTEGER NOT NULL,                -- 0-based, preserves Vec order on read
    name     TEXT    NOT NULL,                -- ingredient name
    qty      TEXT                             -- freetext quantity, NULL if none
) STRICT;

-- Every ingredient read/delete is scoped to one meal_id.
CREATE INDEX idx_meal_ingredients_meal_id ON meal_ingredients (meal_id);

CREATE TABLE grocery_lists (
    id         TEXT NOT NULL PRIMARY KEY,     -- UUID v7
    name       TEXT NOT NULL,                 -- non-empty (validated by GroceryListBuilder)
    created_at TEXT NOT NULL                  -- RFC 3339
) STRICT;

CREATE TABLE grocery_items (
    id             TEXT    NOT NULL PRIMARY KEY,  -- UUID v7
    list_id        TEXT    NOT NULL REFERENCES grocery_lists (id),
    name           TEXT    NOT NULL,              -- non-empty (validated by GroceryItemBuilder)
    qty            TEXT,                          -- freetext quantity, NULL if none
    category       TEXT,                          -- free-form, NULL if uncategorised
    checked        INTEGER NOT NULL,              -- 0/1
    source         TEXT    NOT NULL,              -- snake_case GrocerySource
    source_meal_id TEXT,                          -- MealId, NULL unless source = from_meal
    created_at     TEXT    NOT NULL               -- RFC 3339
) STRICT;

-- Every item read/checked-toggle/delete is scoped to one list_id.
CREATE INDEX idx_grocery_items_list_id ON grocery_items (list_id);

CREATE TABLE pantry_items (
    id         TEXT NOT NULL PRIMARY KEY,     -- UUID v7
    name       TEXT NOT NULL,                 -- non-empty (validated by PantryItemBuilder)
    note       TEXT,                          -- free-form, NULL if none
    created_at TEXT NOT NULL                  -- RFC 3339
) STRICT;

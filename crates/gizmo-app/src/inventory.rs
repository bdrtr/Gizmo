//! [`Inventory`] — what something carries, in slots of stacks; [`ItemCatalog`] — what each item is.
//!
//! ```
//! use gizmo_app::inventory::{Inventory, ItemCatalog, ItemInfo};
//!
//! let mut catalog = ItemCatalog::default();
//! catalog.register("wood", ItemInfo::new("Wood", 20));
//! catalog.register("crystal", ItemInfo::new("Crystal", 99));
//!
//! let mut bag = Inventory::new(4);
//! assert_eq!(bag.add(&catalog, "wood", 30), 0); // 20 in one slot, 10 in the next
//! assert_eq!(bag.count("wood"), 30);
//! assert!(bag.remove("wood", 25));
//! assert_eq!(bag.count("wood"), 5);
//! ```
//!
//! An item is named by a string key, so a game's items are data — written in a file, saved with a
//! scene, sent over the network — and the catalog says what each key is: its name and how many fit
//! in one slot. The inventory is a component (saved with the scene) and serialises on its own, so a
//! game keeps a player's between runs with [`SaveData`](crate::save::SaveData) as it is.
//!
//! Making one thing out of others is a [`Recipe`]: [`Inventory::craft`] takes the inputs and adds
//! the output, all or nothing — nothing is spent when an input is short or the output would not
//! fit.
//!
//! What it does not decide: what an item *does* (eating, equipping, building are the game's), and
//! how it is drawn — `gizmo-ui`'s `Display::Grid`, `Drag` and `DropTarget` are what a slot grid is
//! built from, and [`Inventory::move_stack`] is what a drop does.

use std::collections::HashMap;

/// What an item is: its name and how many fit in one slot.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ItemInfo {
    /// The name shown to the player.
    pub name: String,
    /// The most one slot holds; at least 1.
    pub max_stack: u32,
}

impl ItemInfo {
    /// An item called `name`, up to `max_stack` in a slot.
    #[must_use]
    pub fn new(name: impl Into<String>, max_stack: u32) -> Self {
        Self { name: name.into(), max_stack: max_stack.max(1) }
    }
}

/// Every item a game has, by key — a resource.
#[derive(Clone, Debug, Default)]
pub struct ItemCatalog {
    items: HashMap<String, ItemInfo>,
}

impl ItemCatalog {
    /// How many fit in a slot of an item the catalog does not know.
    pub const DEFAULT_MAX_STACK: u32 = 64;

    /// Says what `key` is.
    pub fn register(&mut self, key: impl Into<String>, info: ItemInfo) {
        self.items.insert(key.into(), info);
    }

    /// What `key` is, if registered.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&ItemInfo> {
        self.items.get(key)
    }

    /// How many of `key` fit in one slot ([`DEFAULT_MAX_STACK`](Self::DEFAULT_MAX_STACK) if unknown).
    #[must_use]
    pub fn max_stack(&self, key: &str) -> u32 {
        self.items.get(key).map_or(Self::DEFAULT_MAX_STACK, |i| i.max_stack)
    }

    /// The name to show for `key`: its registered name, or the key itself.
    #[must_use]
    pub fn name<'a>(&'a self, key: &'a str) -> &'a str {
        self.items.get(key).map_or(key, |i| i.name.as_str())
    }
}

/// Some of one item in one slot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ItemStack {
    /// Which item (an [`ItemCatalog`] key).
    pub item: String,
    /// How many; never 0 in a slot.
    pub count: u32,
}

impl ItemStack {
    /// `count` of `item`.
    #[must_use]
    pub fn new(item: impl Into<String>, count: u32) -> Self {
        Self { item: item.into(), count }
    }
}

/// What something carries: a fixed number of slots, each empty or holding one [`ItemStack`].
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Inventory {
    slots: Vec<Option<ItemStack>>,
}

impl Inventory {
    /// `slots` empty slots.
    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self { slots: vec![None; slots] }
    }

    /// Every slot, in order.
    #[must_use]
    pub fn slots(&self) -> &[Option<ItemStack>] {
        &self.slots
    }

    /// One slot; `None` when empty or past the end.
    #[must_use]
    pub fn slot(&self, index: usize) -> Option<&ItemStack> {
        self.slots.get(index)?.as_ref()
    }

    /// How many slots there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether every slot is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// How many of `item` it holds, over every slot.
    #[must_use]
    pub fn count(&self, item: &str) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    /// Whether it holds at least `count` of `item`.
    #[must_use]
    pub fn has(&self, item: &str, count: u32) -> bool {
        self.count(item) >= count
    }

    /// Puts `count` of `item` in: first onto the stacks of it already there, up to the catalog's
    /// stack size, then into empty slots in order. Returns how many did not fit (0 when all did).
    pub fn add(&mut self, catalog: &ItemCatalog, item: &str, count: u32) -> u32 {
        let max = catalog.max_stack(item);
        let mut left = count;
        for stack in self.slots.iter_mut().flatten().filter(|s| s.item == item) {
            let room = max.saturating_sub(stack.count).min(left);
            stack.count += room;
            left -= room;
        }
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            if left == 0 {
                break;
            }
            let n = left.min(max);
            *slot = Some(ItemStack::new(item, n));
            left -= n;
        }
        left
    }

    /// Takes `count` of `item` out, from the last slots first — or nothing at all, and `false`, when
    /// it holds fewer.
    pub fn remove(&mut self, item: &str, count: u32) -> bool {
        if !self.has(item, count) {
            return false;
        }
        let mut left = count;
        for slot in self.slots.iter_mut().rev() {
            if left == 0 {
                break;
            }
            if let Some(stack) = slot.as_mut().filter(|s| s.item == item) {
                let n = stack.count.min(left);
                stack.count -= n;
                left -= n;
                if stack.count == 0 {
                    *slot = None;
                }
            }
        }
        true
    }

    /// Empties slot `index` and returns what was in it.
    pub fn take(&mut self, index: usize) -> Option<ItemStack> {
        self.slots.get_mut(index)?.take()
    }

    /// What a drop does: moves slot `from` onto slot `to`. Into an empty slot it moves; onto the same
    /// item it adds as much as the stack size allows and leaves the rest where it was; onto another
    /// item the two change places. Returns whether anything changed.
    pub fn move_stack(&mut self, catalog: &ItemCatalog, from: usize, to: usize) -> bool {
        if from == to || from >= self.slots.len() || to >= self.slots.len() {
            return false;
        }
        let Some(moving) = self.slots[from].take() else { return false };
        match self.slots[to].take() {
            None => self.slots[to] = Some(moving),
            Some(mut there) if there.item == moving.item => {
                let room = catalog.max_stack(&there.item).saturating_sub(there.count);
                let n = room.min(moving.count);
                there.count += n;
                self.slots[to] = Some(there);
                if moving.count > n {
                    self.slots[from] = Some(ItemStack::new(moving.item, moving.count - n));
                }
                return n > 0;
            }
            Some(other) => {
                self.slots[to] = Some(moving);
                self.slots[from] = Some(other);
            }
        }
        true
    }
}

gizmo_core::impl_component!(Inventory);

/// How to make something: what goes in and what comes out.
///
/// ```
/// use gizmo_app::inventory::{Inventory, ItemCatalog, Recipe};
///
/// let catalog = ItemCatalog::default();
/// let campfire = Recipe::new("campfire", 1).needs("wood", 4).needs("stone", 2);
/// let mut bag = Inventory::new(8);
/// bag.add(&catalog, "wood", 5);
/// assert!(bag.craft(&catalog, &campfire).is_err()); // no stone
/// bag.add(&catalog, "stone", 2);
/// assert!(bag.craft(&catalog, &campfire).is_ok());
/// assert_eq!((bag.count("wood"), bag.count("stone"), bag.count("campfire")), (1, 0, 1));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Recipe {
    /// What is used up; an item named twice needs both counts.
    pub inputs: Vec<ItemStack>,
    /// What is made.
    pub output: ItemStack,
}

impl Recipe {
    /// A recipe making `count` of `item` out of nothing yet — add inputs with [`needs`](Self::needs).
    #[must_use]
    pub fn new(item: impl Into<String>, count: u32) -> Self {
        Self { inputs: Vec::new(), output: ItemStack::new(item, count) }
    }

    /// This recipe, also using up `count` of `item`.
    #[must_use]
    pub fn needs(mut self, item: impl Into<String>, count: u32) -> Self {
        self.inputs.push(ItemStack::new(item, count));
        self
    }

    /// How many of `item` the recipe uses up, over all its inputs.
    #[must_use]
    pub fn needed(&self, item: &str) -> u32 {
        self.inputs.iter().filter(|s| s.item == item).map(|s| s.count).sum()
    }
}

/// Why [`Inventory::craft`] made nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CraftError {
    /// An input is short: the inventory holds `have` of `item`, the recipe needs `need`.
    Missing {
        /// The item.
        item: String,
        /// How many there are.
        have: u32,
        /// How many the recipe uses.
        need: u32,
    },
    /// The inputs are there, but the output would not fit once they were taken.
    NoRoom,
}

impl std::fmt::Display for CraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { item, have, need } => write!(f, "{need} {item} needed, {have} held"),
            Self::NoRoom => write!(f, "no room for what it makes"),
        }
    }
}

impl std::error::Error for CraftError {}

impl Inventory {
    /// What [`craft`](Self::craft) would say, without making anything.
    ///
    /// # Errors
    /// The same as `craft`.
    pub fn check_craft(&self, catalog: &ItemCatalog, recipe: &Recipe) -> Result<(), CraftError> {
        let mut trial = self.clone();
        trial.craft(catalog, recipe)
    }

    /// Makes `recipe`: takes its inputs out and puts its output in — or, when an input is short
    /// or the output would not fit in what the inputs leave free, changes nothing and says why.
    /// The output stacks like any [`add`](Self::add), so it may land in a slot an input emptied.
    ///
    /// # Errors
    /// [`CraftError::Missing`] for the first input that is short, [`CraftError::NoRoom`] when
    /// the output does not fit.
    pub fn craft(&mut self, catalog: &ItemCatalog, recipe: &Recipe) -> Result<(), CraftError> {
        for input in &recipe.inputs {
            let need = recipe.needed(&input.item);
            let have = self.count(&input.item);
            if have < need {
                return Err(CraftError::Missing { item: input.item.clone(), have, need });
            }
        }
        let mut after = self.clone();
        for input in &recipe.inputs {
            after.remove(&input.item, input.count);
        }
        if after.add(catalog, &recipe.output.item, recipe.output.count) > 0 {
            return Err(CraftError::NoRoom);
        }
        *self = after;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ItemCatalog {
        let mut c = ItemCatalog::default();
        c.register("wood", ItemInfo::new("Wood", 20));
        c.register("sword", ItemInfo::new("Sword", 1));
        c
    }

    /// Adding fills the stacks already there before taking an empty slot, and what does not fit is
    /// handed back.
    #[test]
    fn adding_fills_stacks_then_slots_and_returns_the_rest() {
        let c = catalog();
        let mut bag = Inventory::new(3);
        assert_eq!(bag.add(&c, "wood", 15), 0);
        assert_eq!(bag.add(&c, "sword", 1), 0);
        assert_eq!(bag.add(&c, "wood", 30), 5, "20 + 5 more fit in the third slot... and 5 do not");
        assert_eq!(bag.slot(0), Some(&ItemStack::new("wood", 20)));
        assert_eq!(bag.slot(1), Some(&ItemStack::new("sword", 1)));
        assert_eq!(bag.slot(2), Some(&ItemStack::new("wood", 20)));
        assert_eq!(bag.count("wood"), 40);
        assert_eq!(bag.add(&c, "sword", 1), 1, "full");
    }

    /// Removing takes all or nothing, from the last slots first, and empties the slots it drains.
    #[test]
    fn removing_is_all_or_nothing() {
        let c = catalog();
        let mut bag = Inventory::new(3);
        bag.add(&c, "wood", 30);
        assert!(!bag.remove("wood", 31));
        assert_eq!(bag.count("wood"), 30, "a failed remove took nothing");
        assert!(bag.remove("wood", 12));
        assert_eq!(bag.slot(0), Some(&ItemStack::new("wood", 18)));
        assert_eq!(bag.slot(1), None);
        assert!(bag.remove("wood", 18));
        assert!(bag.is_empty());
    }

    /// A drop moves into an empty slot, tops up a stack of the same item (leaving what does not fit),
    /// and swaps with another item.
    #[test]
    fn a_drop_moves_merges_or_swaps() {
        let c = catalog();
        let mut bag = Inventory::new(4);
        bag.add(&c, "wood", 20);
        bag.add(&c, "wood", 8);
        bag.add(&c, "sword", 1);
        // [wood 20, wood 8, sword, _]
        assert!(bag.move_stack(&c, 2, 3));
        assert_eq!(bag.slot(3), Some(&ItemStack::new("sword", 1)));
        assert!(bag.move_stack(&c, 0, 1), "8 + 12 of the 20 fit");
        assert_eq!(bag.slot(1), Some(&ItemStack::new("wood", 20)));
        assert_eq!(bag.slot(0), Some(&ItemStack::new("wood", 8)));
        assert!(bag.move_stack(&c, 3, 0), "different items change places");
        assert_eq!(bag.slot(0), Some(&ItemStack::new("sword", 1)));
        assert_eq!(bag.slot(3), Some(&ItemStack::new("wood", 8)));
        assert!(!bag.move_stack(&c, 2, 1), "an empty slot moves nothing");
        assert!(!bag.move_stack(&c, 1, 9), "past the end");
        assert_eq!(bag.count("wood"), 28);
    }

    /// An unknown item stacks to the default size; the catalog names it by its key.
    #[test]
    fn an_unknown_item_takes_the_default_stack() {
        let c = catalog();
        let mut bag = Inventory::new(2);
        assert_eq!(bag.add(&c, "pebble", 100), 0);
        assert_eq!(bag.slot(0).unwrap().count, ItemCatalog::DEFAULT_MAX_STACK);
        assert_eq!(c.name("pebble"), "pebble");
        assert_eq!(c.name("wood"), "Wood");
    }

    /// A recipe takes its inputs and gives its output — and with an input short, or no room for
    /// the output, takes nothing.
    #[test]
    fn crafting_is_all_or_nothing() {
        let c = catalog(); // wood stacks to 20, swords to 1
        let sword = Recipe::new("sword", 1).needs("wood", 3).needs("wood", 2);
        assert_eq!(sword.needed("wood"), 5);
        let mut bag = Inventory::new(2);
        bag.add(&c, "wood", 4);
        assert_eq!(bag.craft(&c, &sword), Err(CraftError::Missing { item: "wood".into(), have: 4, need: 5 }));
        assert_eq!(bag.count("wood"), 4, "a failed craft spent wood");
        bag.add(&c, "wood", 21); // 20 + 5: both slots taken
        bag.add(&c, "sword", 0);
        assert_eq!(bag.check_craft(&c, &sword), Ok(()), "five of 25 wood leave the second slot free");
        assert!(bag.craft(&c, &sword).is_ok());
        assert_eq!((bag.count("wood"), bag.count("sword")), (20, 1));
        // Now both slots hold something the next sword cannot share: no room.
        assert_eq!(bag.craft(&c, &sword), Err(CraftError::NoRoom));
        assert_eq!((bag.count("wood"), bag.count("sword")), (20, 1), "a craft with no room spent wood");
        // The output may take the slot an input emptied.
        let mut tight = Inventory::new(1);
        tight.add(&c, "wood", 5);
        assert!(tight.craft(&c, &sword).is_ok());
        assert_eq!(tight.slot(0), Some(&ItemStack::new("sword", 1)));
    }

    /// It round-trips through JSON, which is what `SaveData` keeps.
    #[test]
    fn an_inventory_round_trips_through_json() {
        let c = catalog();
        let mut bag = Inventory::new(3);
        bag.add(&c, "wood", 25);
        let text = serde_json::to_string(&bag).unwrap();
        assert_eq!(serde_json::from_str::<Inventory>(&text).unwrap(), bag);
    }
}

//! Factorio-style autocraft over native settlement production.
//!
//! The owner picks one target recipe. [`plan`] walks its inputs against live
//! town stock (unreserved storage plus the worker's carry and equipment);
//! every shortfall is made by that item's default recipe, recursively, and
//! the result is a post-order chain of [`Step`]s. [`order`] admits the chain
//! atomically; production then runs it step by step with the same storage
//! fetches, station reach, portions and receipts as any other job. A carry
//! too full for the next portion first walks its surplus (an earlier step's
//! intermediates included) into town storage, so long chains never wedge on
//! their own output. Planning moves and reserves nothing: stock that
//! disappears before a step runs parks that step honestly ("missing input"),
//! it is never fabricated.

use mc_data::ItemStack;
use mc_data::recipes::work::{WorkRecipe, WorkRecipeInput};
use mc_script::{ScriptOperationFailure as Failure, ScriptSettlementBuildingRef};
use uuid::Uuid;

use super::buildings::BuildingKind;
use super::production::{self, MAX_ORDER_STEPS, Step};
use super::ui_text::{self, Key, Lang};
use super::{SettlementId, economy, inventory};
use crate::tick::TickState;

/// Recipe nesting depth one order may walk.
const MAX_DEPTH: usize = 16;
/// Crafts one step may run.
const MAX_PORTIONS: u64 = 4096;

/// Owner request: run `recipe` `count` times at `station`, crafting every
/// missing ingredient first. `job` becomes the final step's id.
pub(crate) struct Order {
    pub job: String,
    pub worker: Uuid,
    pub recipe: String,
    pub count: u32,
    pub building: ScriptSettlementBuildingRef,
    pub station: [i32; 3],
    pub stand_at: [i32; 3],
}

/// Why an order was not admitted. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    UnknownRecipe(String),
    /// No stock and no recipe produces the item.
    Missing {
        item: String,
        count: u64,
    },
    /// The only way to make the item leads back into a recipe already being
    /// expanded (for example wheat -> hay block -> wheat).
    Cycle {
        item: String,
        count: u64,
        recipes: Vec<String>,
    },
    Tool {
        recipe: String,
        tool: String,
    },
    Station {
        recipe: String,
        station: String,
    },
    TooLarge,
    Exists(String),
    /// Native admission refused the planned chain.
    Admission(Failure),
}

impl Refusal {
    /// Player-facing reason in the dialog language.
    pub(crate) fn text(&self, lang: Lang) -> String {
        let say = |key: Key, args: &[(&str, &str)]| fill(lang, key, args);
        match self {
            Self::UnknownRecipe(recipe) => {
                say(Key::AutoUnknownRecipe, &[("recipe", recipe.as_str())])
            }
            Self::Missing { item, count } => say(
                Key::AutoMissing,
                &[
                    ("count", count.to_string().as_str()),
                    ("item", item.as_str()),
                ],
            ),
            Self::Cycle {
                item,
                count,
                recipes,
            } => say(
                Key::AutoCycle,
                &[
                    ("count", count.to_string().as_str()),
                    ("item", item.as_str()),
                    ("recipes", recipes.join(" -> ").as_str()),
                ],
            ),
            Self::Tool { recipe, tool } => say(
                Key::AutoTool,
                &[("recipe", recipe.as_str()), ("tool", tool.as_str())],
            ),
            Self::Station { recipe, station } => say(
                Key::AutoStation,
                &[("recipe", recipe.as_str()), ("station", station.as_str())],
            ),
            Self::TooLarge => say(
                Key::AutoTooLarge,
                &[
                    ("steps", MAX_ORDER_STEPS.to_string().as_str()),
                    ("portions", MAX_PORTIONS.to_string().as_str()),
                ],
            ),
            Self::Exists(job) => say(Key::AutoExists, &[("job", job.as_str())]),
            Self::Admission(failure) => format!("{failure:?}"),
        }
    }
}

fn fill(lang: Lang, key: Key, args: &[(&str, &str)]) -> String {
    ui_text::fill(ui_text::text(lang, key), args)
}

/// `recipe x3 -> recipe x1` in chain order.
pub(crate) fn describe(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|step| {
            let name = step.recipe.rsplit(':').next().unwrap_or(&step.recipe);
            format!("{name} x{}", step.count)
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn label(input: &WorkRecipeInput) -> String {
    match (&input.item, &input.tag) {
        (Some(item), _) => item.to_string(),
        (None, Some(tag)) => format!("#{tag}"),
        (None, None) => "ingredient".to_owned(),
    }
}

/// Counted stock the plan may still draw from, in draw order.
struct Stock(Vec<(ItemStack, u64)>);

impl Stock {
    fn take(&mut self, world: &TickState, input: &WorkRecipeInput, want: u64) -> u64 {
        let mut taken = 0;
        for (stack, available) in &mut self.0 {
            if taken == want {
                break;
            }
            if *available > 0 && production::accepts(world, stack, input) {
                let take = (want - taken).min(*available);
                *available -= take;
                taken += take;
            }
        }
        taken
    }
}

/// The worker's carry and equipment (production draws those first), then the
/// free share of every live town container, each physical container once
/// (a double chest is one inventory, not two): the same sources and
/// reservation floors production fetches from. Also returns what the worker
/// holds.
fn stock(
    world: &TickState,
    settlement: SettlementId,
    worker: Uuid,
) -> Result<(Stock, Vec<ItemStack>), Failure> {
    let mut stacks = Vec::new();
    for endpoint in [
        inventory::Endpoint::VillagerCarry { uuid: worker },
        inventory::Endpoint::VillagerEquipment { uuid: worker },
    ] {
        for stack in inventory::load(world, &endpoint)?.slots {
            if !stack.is_empty() {
                let count = stack.count.max(0) as u64;
                stacks.push((stack, count));
            }
        }
    }
    let held = stacks.iter().map(|(stack, _)| stack.clone()).collect();
    for (_, image) in inventory::town_storage(world, settlement) {
        let Ok(mut unreserved) = inventory::unreserved_stock(world, &image) else {
            continue;
        };
        for stack in image.slots {
            if stack.is_empty() {
                continue;
            }
            let Some(free) = unreserved.get_mut(&stack.item_id) else {
                continue;
            };
            let count = (stack.count.max(0) as u64).min(*free);
            *free -= count;
            if count > 0 {
                stacks.push((stack, count));
            }
        }
    }
    Ok((Stock(stacks), held))
}

struct Planner<'a> {
    world: &'a TickState,
    stock: Stock,
    /// Carry + equipment at planning time: tools are worn, never planned.
    held: Vec<ItemStack>,
    path: Vec<&'a WorkRecipe>,
    steps: Vec<(&'a WorkRecipe, u64)>,
}

impl<'a> Planner<'a> {
    fn run(&mut self, recipe: &'a WorkRecipe, portions: u64) -> Result<(), Refusal> {
        if portions > MAX_PORTIONS || self.path.len() >= MAX_DEPTH {
            return Err(Refusal::TooLarge);
        }
        for tool in &recipe.tools {
            let held = self.held.iter().any(|stack| {
                production::resource(self.world, stack) == Some(tool.item.as_str())
                    && tool
                        .components
                        .as_ref()
                        .is_none_or(|components| components.matches(stack))
            });
            if !held {
                return Err(Refusal::Tool {
                    recipe: recipe.id.path().to_owned(),
                    tool: tool.item.to_string(),
                });
            }
        }
        self.path.push(recipe);
        for input in &recipe.inputs {
            self.need(input, u64::from(input.count) * portions)?;
        }
        self.path.pop();
        for output in recipe.outputs.iter().chain(&recipe.remainders) {
            let stack = output
                .to_stack(&self.world.registry.items, &self.world.registry.item_facts)
                .ok_or_else(|| Refusal::UnknownRecipe(recipe.id.to_string()))?;
            self.stock
                .0
                .push((stack, u64::from(output.count) * portions));
        }
        // Consecutive crafts of one recipe are one step.
        if self
            .steps
            .last()
            .is_some_and(|(last, _)| last.id == recipe.id)
        {
            self.steps.last_mut().expect("merged step").1 += portions;
        } else {
            self.steps.push((recipe, portions));
        }
        if self.steps.len() > MAX_ORDER_STEPS
            || self
                .steps
                .last()
                .is_some_and(|(_, count)| *count > MAX_PORTIONS)
        {
            return Err(Refusal::TooLarge);
        }
        Ok(())
    }

    fn need(&mut self, input: &'a WorkRecipeInput, quantity: u64) -> Result<(), Refusal> {
        let short = quantity - self.stock.take(self.world, input, quantity);
        if short == 0 {
            return Ok(());
        }
        let Some((recipe, made)) = self.producer(input) else {
            return Err(Refusal::Missing {
                item: label(input),
                count: short,
            });
        };
        if let Some(start) = self.path.iter().position(|open| open.id == recipe.id) {
            let recipes = self.path[start..]
                .iter()
                .chain(std::iter::once(&recipe))
                .map(|open| open.id.path().to_owned())
                .collect();
            return Err(Refusal::Cycle {
                item: label(input),
                count: short,
                recipes,
            });
        }
        self.run(recipe, short.div_ceil(made))?;
        if self.stock.take(self.world, input, short) < short {
            return Err(Refusal::Missing {
                item: label(input),
                count: short,
            });
        }
        Ok(())
    }

    /// The item's default recipe and its yield per craft. Recipes that rework
    /// the same item (repair) never make it; a recipe named after its product
    /// (`stick`, `wooden_pickaxe_ordinary`) wins over by-products (`salvage_*`),
    /// then identifier order.
    fn producer(&self, input: &WorkRecipeInput) -> Option<(&'a WorkRecipe, u64)> {
        let world = self.world;
        world
            .registry
            .item_facts
            .work_recipes()
            .filter_map(|recipe| {
                let reworks = recipe.inputs.iter().any(|own| {
                    recipe
                        .outputs
                        .iter()
                        .any(|output| own.item.as_ref() == Some(&output.item))
                });
                let mut named = false;
                let mut made = 0;
                for output in &recipe.outputs {
                    let accepted = output
                        .to_stack(&world.registry.items, &world.registry.item_facts)
                        .is_some_and(|stack| production::accepts(world, &stack, input));
                    if accepted {
                        made += u64::from(output.count);
                        named |= recipe.id.path().starts_with(output.item.path());
                    }
                }
                (made > 0 && !reworks).then_some((recipe, made, named))
            })
            .min_by_key(|(_, _, named)| !named)
            .map(|(recipe, made, _)| (recipe, made))
    }
}

/// A live station for `recipe`: the requested one when its block matches,
/// else the first matching block (with a free stand cell) in a certified
/// workshop of this town.
fn station_for(
    world: &TickState,
    settlement: SettlementId,
    recipe: &WorkRecipe,
    order: &Order,
) -> Result<(ScriptSettlementBuildingRef, [i32; 3], [i32; 3]), Refusal> {
    if production::station_block(world, recipe, order.station).is_ok() {
        return Ok((order.building.clone(), order.station, order.stand_at));
    }
    let workshops = world
        .settlements
        .get(settlement)
        .into_iter()
        .flat_map(|record| record.buildings.values())
        .filter(|building| matches!(building.kind, BuildingKind::Workshop(_)));
    for building in workshops {
        let bounds = building.bounds;
        for y in bounds.min[1]..=bounds.max[1] {
            for z in bounds.min[2]..=bounds.max[2] {
                for x in bounds.min[0]..=bounds.max[0] {
                    if production::station_block(world, recipe, [x, y, z]).is_err() {
                        continue;
                    }
                    if let Some(stand) = production::stand_at(world, [x, y, z], bounds) {
                        let reference = ScriptSettlementBuildingRef::new(
                            "building".to_owned(),
                            building.id.0.to_string(),
                            format!("krasnokamen:{}", building.kind.name()),
                        );
                        return Ok((reference, [x, y, z], stand));
                    }
                }
            }
        }
    }
    Err(Refusal::Station {
        recipe: recipe.id.path().to_owned(),
        station: recipe.station.to_string(),
    })
}

/// Plan the chain for `order` against live stock. The final step is always
/// the requested recipe `count` times; every earlier step makes a shortfall.
pub(crate) fn plan(
    world: &TickState,
    settlement: SettlementId,
    order: &Order,
) -> Result<Vec<Step>, Refusal> {
    if order.count == 0 {
        return Err(Refusal::Admission(Failure::InvalidRequest));
    }
    production::worker_slot(world, settlement, order.worker).map_err(Refusal::Admission)?;
    let target = production::recipe_of(world, &order.recipe)
        .map_err(|_| Refusal::UnknownRecipe(order.recipe.clone()))?;
    let (stock, held) = stock(world, settlement, order.worker).map_err(Refusal::Admission)?;
    let mut planner = Planner {
        world,
        stock,
        held,
        path: Vec::new(),
        steps: Vec::new(),
    };
    planner.run(target, u64::from(order.count))?;
    planner
        .steps
        .into_iter()
        .map(|(recipe, portions)| -> Result<Step, Refusal> {
            let (building, station, stand_at) = station_for(world, settlement, recipe, order)?;
            Ok(Step {
                recipe: recipe.id.to_string(),
                count: u32::try_from(portions).map_err(|_| Refusal::TooLarge)?,
                station,
                stand_at,
                building,
            })
        })
        .collect()
}

/// Plan and admit one order through the native economy boundary (owner,
/// revision and WAL handling). A known order id is refused, never re-planned,
/// so retries cannot duplicate work.
pub(crate) fn order(
    world: &mut TickState,
    settlement: SettlementId,
    order: Order,
) -> Result<Vec<Step>, Refusal> {
    let exists = world
        .settlements
        .get(settlement)
        .ok_or(Refusal::Admission(Failure::NotFound))?
        .economy
        .production
        .jobs
        .contains_key(&order.job);
    if exists {
        return Err(Refusal::Exists(order.job));
    }
    let steps = plan(world, settlement, &order)?;
    economy::execute(
        world,
        settlement,
        economy::Request::Production(production::Request::Order {
            order: order.job,
            worker: order.worker,
            steps: steps.clone(),
        }),
    )
    .map_err(Refusal::Admission)?;
    Ok(steps)
}

#[cfg(test)]
#[path = "autocraft_tests.rs"]
mod tests;

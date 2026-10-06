use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Tag, Taggable};

/// Which entity types a target or threat search accepts.
#[derive(Clone, Copy, Debug)]
pub enum TargetMatch {
    /// Every type. The goal's predicate does the filtering.
    Any,
    Type(&'static EntityType),
    AnyOf(&'static [&'static EntityType]),
    Tag(&'static Tag),
    /// Every type outside the tag. e.g. Wither: `wither_friends`.
    AllExcept(&'static Tag),
}

impl TargetMatch {
    #[must_use]
    pub fn matches(self, entity_type: &EntityType) -> bool {
        match self {
            Self::Any => true,
            Self::Type(t) => t == entity_type,
            Self::AnyOf(types) => types.contains(&entity_type),
            Self::Tag(tag) => entity_type.has_tag(tag),
            Self::AllExcept(tag) => !entity_type.has_tag(tag),
        }
    }

    /// Only players can match -> the search can use the player list.
    #[must_use]
    pub fn is_player(self) -> bool {
        matches!(self, Self::Type(t) if t == &EntityType::PLAYER)
    }
}

/// Mob families vanilla goals refer to by class. A tag is used only where it has
/// exactly the class's members: `skeletons` adds the skeleton horse, `zombies` adds horses and zoglin.
impl TargetMatch {
    pub const UNDEAD: Self = Self::Tag(&tag::EntityType::MINECRAFT_UNDEAD);
    /// `AbstractIllager`.
    pub const ILLAGER: Self = Self::Tag(&tag::EntityType::MINECRAFT_ILLAGER);
    pub const CAT_LIKE: Self = Self::AnyOf(&[&EntityType::CAT, &EntityType::OCELOT]);
    /// `AbstractSchoolingFish`: the ones that swim in schools.
    pub const FISH: Self = Self::AnyOf(&[
        &EntityType::COD,
        &EntityType::SALMON,
        &EntityType::TROPICAL_FISH,
    ]);
    pub const GUARDIAN_LIKE: Self =
        Self::AnyOf(&[&EntityType::GUARDIAN, &EntityType::ELDER_GUARDIAN]);
    pub const SQUID_LIKE: Self = Self::AnyOf(&[&EntityType::SQUID, &EntityType::GLOW_SQUID]);
    /// `AbstractVillager`.
    pub const VILLAGER_LIKE: Self =
        Self::AnyOf(&[&EntityType::VILLAGER, &EntityType::WANDERING_TRADER]);
    /// `Zombie`.
    pub const ZOMBIE_LIKE: Self = Self::AnyOf(&[
        &EntityType::ZOMBIE,
        &EntityType::HUSK,
        &EntityType::DROWNED,
        &EntityType::ZOMBIE_VILLAGER,
        &EntityType::ZOMBIFIED_PIGLIN,
    ]);
    /// `AbstractSkeleton`.
    pub const SKELETON_LIKE: Self = Self::AnyOf(&[
        &EntityType::SKELETON,
        &EntityType::STRAY,
        &EntityType::WITHER_SKELETON,
        &EntityType::BOGGED,
        &EntityType::PARCHED,
    ]);
    /// `AbstractPiglin`.
    pub const PIGLIN_LIKE: Self = Self::AnyOf(&[&EntityType::PIGLIN, &EntityType::PIGLIN_BRUTE]);
    /// A piglin's nemesis: wither skeleton or wither, whichever is closest.
    pub const WITHER_LIKE: Self = Self::AnyOf(&[&EntityType::WITHER_SKELETON, &EntityType::WITHER]);
    /// `Llama`.
    pub const LLAMA_LIKE: Self = Self::AnyOf(&[&EntityType::LLAMA, &EntityType::TRADER_LLAMA]);
}

impl From<&'static EntityType> for TargetMatch {
    fn from(entity_type: &'static EntityType) -> Self {
        Self::Type(entity_type)
    }
}

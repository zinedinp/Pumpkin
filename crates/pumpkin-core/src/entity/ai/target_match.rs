use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{Tag, Taggable};

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

impl From<&'static EntityType> for TargetMatch {
    fn from(entity_type: &'static EntityType) -> Self {
        Self::Type(entity_type)
    }
}

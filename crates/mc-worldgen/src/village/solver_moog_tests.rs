use super::*;

fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}

fn pool(id_value: &str, entries: Vec<(u32, ClosureElement)>) -> ClosurePool {
    ClosurePool {
        id: id(id_value),
        fallback: id("minecraft:empty"),
        max_size: 1,
        elements: entries,
    }
}

fn feature(name: &str) -> ClosureElement {
    ClosureElement::Feature {
        placed_feature: id(name),
        projection: Projection::Rigid,
    }
}

#[test]
fn moog_candidates_draw_raw_weighted_entries_without_replacement() {
    let p = pool(
        "mvs:pool",
        vec![(1, feature("mvs:a")), (5, feature("mvs:b"))],
    );
    let mut candidates = WeightedCandidates::new(&p);
    let mut random = LegacyRandom::new(45);
    let first = candidates.next(&mut random).unwrap();
    let second = candidates.next(&mut random).unwrap();
    assert_ne!(first, second, "a weighted raw entry is not attempted twice");
    assert!(candidates.next(&mut random).is_none());
}

#[test]
fn moog_empty_primary_advances_to_weighted_fallback() {
    let primary = pool("mvs:primary", vec![(1, ClosureElement::Empty)]);
    let fallback = pool("mvs:fallback", vec![(1, feature("mvs:lower"))]);
    let mut candidates = CandidateStream::Moog {
        primary: Some(WeightedCandidates::new(&primary)),
        fallback: Some(WeightedCandidates::new(&fallback)),
    };
    let mut random = LegacyRandom::new(7);
    assert!(
        matches!(candidates.next(&mut random), Some(PlacedElement::Feature { placed_feature, .. }) if placed_feature == id("mvs:lower"))
    );
    assert!(candidates.next(&mut random).is_none());
}

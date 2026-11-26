use super::*;

#[test]
fn ordinary_lanes_are_all_player_one_keys() {
    let lanes = Lane::ordinary(4);
    assert_eq!(lanes.len(), 4);
    for lane in &lanes {
        assert_eq!(lane.player, Player::One);
        assert_eq!(lane.role, LaneRole::Key);
    }
}

#[test]
fn sides_marks_only_the_last_lane_of_each_side_as_scratch() {
    let lanes = Lane::sides(6, Some(7), true, false);
    assert_eq!(lanes.len(), 13);
    for lane in &lanes[..5] {
        assert_eq!(lane.player, Player::One);
        assert_eq!(lane.role, LaneRole::Key);
    }
    assert_eq!(lanes[5].player, Player::One);
    assert_eq!(lanes[5].role, LaneRole::Scratch);
    for lane in &lanes[6..] {
        assert_eq!(lane.player, Player::Two);
        assert_eq!(lane.role, LaneRole::Key);
    }
}

#[test]
fn sides_without_a_second_player_omits_player_two_lanes() {
    let lanes = Lane::sides(4, None, false, false);
    assert_eq!(lanes.len(), 4);
    for lane in &lanes {
        assert_eq!(lane.player, Player::One);
    }
}

#[test]
fn binding_layout_separates_sides_with_a_slash() {
    let lanes = Lane::sides(6, Some(7), true, false);
    assert_eq!(Lane::binding_layout(&lanes), "KKKKKS/KKKKKKK");
}

#[test]
fn binding_layout_for_singles_has_no_slash() {
    assert_eq!(Lane::binding_layout(&Lane::ordinary(4)), "KKKK");
}

#[test]
fn bindings_player_two_4k_uses_fixed_layout() {
    let lanes = Lane::sides(0, Some(4), false, false);
    assert_eq!(Lane::bindings(&lanes), single_keys("ZXNM"));
}

#[test]
fn bindings_player_two_5k_without_scratch_uses_fixed_layout() {
    let lanes = Lane::sides(0, Some(5), false, false);
    assert_eq!(Lane::bindings(&lanes), single_keys("ZXCNM"));
}

#[test]
fn bindings_player_two_5k_with_scratch_appends_the_scratch_key() {
    let lanes = Lane::sides(0, Some(6), false, true);
    let mut expected = single_keys("ZXCVB");
    expected.push(vec![';']);
    assert_eq!(Lane::bindings(&lanes), expected);
}

#[test]
fn bindings_player_two_7k_uses_fixed_layout() {
    let lanes = Lane::sides(0, Some(7), false, false);
    assert_eq!(Lane::bindings(&lanes), single_keys("ZXCVBNM"));
}

#[test]
fn bindings_double_play_5k_uses_the_dedicated_player_one_layout() {
    let lanes = Lane::sides(5, Some(7), false, false);
    let mut expected = single_keys("SDFKL");
    expected.extend(single_keys("ZXCVBNM"));
    assert_eq!(Lane::bindings(&lanes), expected);
}

#[test]
fn bindings_falls_back_to_keys_for_unlisted_counts() {
    let lanes = Lane::ordinary(9);
    assert_eq!(Lane::bindings(&lanes), keys_for(9));
}

//! Smoke tests for the multi-agent MCTS API (Issue #42).
//!
//! These tests cover the public surface added to `crates/solver-mcts`
//! by Issue #42: [`MultiAgentMCTS`], [`JointAction`], and
//! [`MultiAgentNode`]. They live in `tests/` so they exercise the same
//! surface downstream crates will import.

use std::time::Duration;

use solver_mcts::Solver;
use solver_mcts::multi_agent::{AgentId, JointAction, MultiAgentMCTS, MultiAgentNode};
use solver_mcts::types::{Action, ActionSpec, DayState};

/// Three-agent fixture whose per-agent pools differ in size so the
/// expected Cartesian-product is non-trivial: 3 × 2 × 4 = 24.
fn fixture_three_agents() -> Vec<DayState> {
    vec![
        DayState::new(
            vec![
                ActionSpec { index: 0, immediate_value: 1.0 },
                ActionSpec { index: 1, immediate_value: 2.0 },
                ActionSpec { index: 2, immediate_value: 3.0 },
            ],
            16,
        ),
        DayState::new(
            vec![
                ActionSpec { index: 3, immediate_value: 4.0 },
                ActionSpec { index: 4, immediate_value: 5.0 },
            ],
            16,
        ),
        DayState::new(
            vec![
                ActionSpec { index: 5, immediate_value: 6.0 },
                ActionSpec { index: 6, immediate_value: 7.0 },
                ActionSpec { index: 7, immediate_value: 8.0 },
                ActionSpec { index: 8, immediate_value: 9.0 },
            ],
            16,
        ),
    ]
}

/// Two-agent fixture with one trivial agent to keep the test surface
/// compact.
fn fixture_two_agents() -> Vec<DayState> {
    vec![
        DayState::new(
            vec![
                ActionSpec { index: 0, immediate_value: 1.0 },
                ActionSpec { index: 1, immediate_value: 5.0 },
                ActionSpec { index: 2, immediate_value: 3.0 },
            ],
            16,
        ),
        DayState::new(
            vec![
                ActionSpec { index: 3, immediate_value: 4.0 },
                ActionSpec { index: 4, immediate_value: 2.0 },
            ],
            16,
        ),
    ]
}

#[test]
fn joint_action_space_size_matches_expected_product() {
    let states = fixture_three_agents();
    let product = states.iter().map(|s| s.legal_actions().len()).product::<usize>();
    // 3 * 2 * 4 = 24
    assert_eq!(product, 24);

    // The crate API reports the same number.
    assert_eq!(MultiAgentMCTS::joint_action_space_size(&states), 24);

    // Enumerating the joint space yields exactly that many joint actions.
    let per_agent: Vec<Vec<Action>> = states.iter().map(DayState::legal_actions).collect();
    let joint = JointAction::enumerate(&per_agent);
    assert_eq!(joint.len(), 24);

    // Every joint action has one entry per agent.
    for ja in &joint {
        assert_eq!(ja.actions().len(), states.len());
    }
}

#[test]
fn ucb_selection_prefers_less_visited_joint_action() {
    // Two competing joint actions with equal per-step reward: UCB must
    // prefer the less-visited one because its exploration term is
    // larger.
    let actions = vec![
        JointAction::new(vec![Action(0), Action(3)]),
        JointAction::new(vec![Action(1), Action(4)]),
    ];

    let mut root = MultiAgentNode::root(actions.clone());

    // First joint-action: well-visited, moderate reward (mean = 0.5).
    let mut heavy = MultiAgentNode::root(Vec::new());
    heavy.visits = 100;
    heavy.total_reward = 50.0;
    root.children.insert(actions[0].clone(), heavy);

    // Second joint-action: lightly visited, same reward (mean = 0.5).
    let mut light = MultiAgentNode::root(Vec::new());
    light.visits = 4;
    light.total_reward = 2.0;
    root.children.insert(actions[1].clone(), light);

    // Anchor root visits so the UCB exploration term is well defined.
    root.visits = 104;

    let best =
        root.best_child_ucb1(root.visits, core::f64::consts::SQRT_2).expect("must pick a child");
    // Exploit is identical; explore favours the lighter node.
    assert_eq!(best, &actions[1]);

    // Strongly contrasting case: an unvisited child must always win.
    let mut root2 = MultiAgentNode::root(actions.clone());
    root2.children.insert(actions[0].clone(), {
        let mut n = MultiAgentNode::root(Vec::new());
        n.visits = 10;
        n.total_reward = 5.0;
        n
    });
    root2.children.insert(actions[1].clone(), MultiAgentNode::root(Vec::new()));
    root2.visits = 10;
    let best2 = root2.best_child_ucb1(10, core::f64::consts::SQRT_2).expect("must pick");
    assert_eq!(best2, &actions[1]);
}

#[test]
fn multi_agent_plan_returns_one_action_per_agent() {
    let states = fixture_two_agents();
    let solver =
        MultiAgentMCTS::new(core::f64::consts::SQRT_2, vec![AgentId(0), AgentId(1)], 256, 42);
    let joint = solver.plan_step(&states, Duration::from_secs(1)).expect("plan must succeed");
    // One action per agent.
    assert_eq!(joint.actions().len(), states.len());
    assert_eq!(joint.actions().len(), solver.agents.len());

    // Every per-agent action is a legal index for its state.
    for (i, a) in joint.actions().iter().enumerate() {
        let legal = states[i].legal_actions();
        assert!(legal.contains(a), "agent {i} action {} not legal in state {i}", a.index());
    }

    // Repeated calls with the same seed are deterministic.
    let joint_b = solver.plan_step(&states, Duration::from_secs(1)).expect("plan must succeed");
    assert_eq!(joint, joint_b);
}

#[test]
fn multi_agent_plan_surfaces_high_value_joint_action() {
    // Three-agent fixture: action (2, 4, 8) is the obvious maximiser
    // (per-agent best = 3.0 + 5.0 + 9.0 = 17.0). The solver should
    // converge to it after enough iterations.
    let states = fixture_three_agents();
    let solver = MultiAgentMCTS::new(
        core::f64::consts::SQRT_2,
        vec![AgentId(0), AgentId(1), AgentId(2)],
        4_096,
        0x42,
    );
    let joint = solver.plan_step(&states, Duration::from_secs(2)).expect("plan must succeed");

    let expected = JointAction::new(vec![Action(2), Action(4), Action(8)]);
    assert_eq!(joint, expected);
}

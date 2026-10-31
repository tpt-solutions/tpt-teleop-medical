//! The feed driven by the running simulator, end to end: simulated joints,
//! tool position, buffered frames, scene snapshot and the HTML export.

use tpt_t_medical_sim::{Fault, JOINTS, JointParams, Simulator, tool_position};
use tpt_t_medical_surgical::kinematics::Arm;
use tpt_t_medical_visualizer::feed::{Feed, Frame};
use tpt_t_medical_visualizer::html;
use tpt_t_medical_visualizer::scene::{Copy3, fit};

#[test]
fn a_running_simulation_reaches_the_screen_through_the_feed() {
    let arm = Arm::reference();
    let params = [JointParams::new(0.5, 0.2, 0.0, 0.0, 0.0).expect("params"); JOINTS];
    let mut sim = Simulator::new(params, [0.0; JOINTS]);
    let mut feed = Feed::new(1_000, 16).expect("feed");
    feed.set_envelope([-0.5; 3], [0.5; 3]);
    feed.push_event(0, "session engaged");

    for step in 0..500_u64 {
        let readings = sim.step(Some([0.4; JOINTS]));
        let tool = tool_position(&arm, &readings);
        feed.push_frame(Frame {
            at_us: readings.time_us,
            tool_m: tool,
            force_n: [0.0; 3],
            tmr: [Copy3::Agree; 3],
        });
        if step == 250 {
            feed.push_event(readings.time_us, "mid-run checkpoint");
        }
    }

    // A sensor fault drops the position for a few ticks. Those ticks are kept in
    // the buffer but left out of the trajectory.
    sim.inject(Fault::SensorFailure(0)).expect("joint");
    for _ in 0..10 {
        let readings = sim.step(Some([0.4; JOINTS]));
        feed.push_frame(Frame {
            at_us: readings.time_us,
            tool_m: tool_position(&arm, &readings),
            force_n: [0.0; 3],
            tmr: [Copy3::Agree; 3],
        });
    }

    let scene = feed.snapshot();
    assert_eq!(
        scene.trajectory_m.len(),
        500,
        "unknown positions are skipped"
    );
    assert_ne!(
        scene.trajectory_m.first(),
        scene.trajectory_m.last(),
        "the tool moved under the commanded torque"
    );
    assert_eq!(scene.events.len(), 2);

    assert!(
        scene.envelope_m.is_some(),
        "the workspace box is carried through"
    );
    let view = fit(&scene.trajectory_m, 800.0, 600.0);
    let page = html::render(&scene, &view, 800, 600);
    assert!(page.contains("mid-run checkpoint"));
    assert!(page.len() > 1_000);
}

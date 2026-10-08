use bench::CaseSpec;
use engine::model::MatchState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let case: CaseSpec = CaseSpec::from_bundled_fixture(&args[1])?;
    let team = domain::PlayerId::new("player-a")?;
    let state = MatchState::new(case.to_engine_setting(&team)?)?;
    let map = &state.setting.map;
    let w = usize::from(map.size.width);
    for row in 0..usize::from(map.size.height) {
        let mut line = Vec::new();
        for col in 0..w {
            let cell = &map.cells()[row * w + col];
            let t = match cell.terrain {
                engine::model::Terrain::Plain => "P".to_string(),
                engine::model::Terrain::Mountain => "M".to_string(),
                engine::model::Terrain::Pond => "~".to_string(),
                engine::model::Terrain::Road(s) => format!("R{s:?}"),
            };
            let spot = if cell.spot.is_some() { "*" } else { " " };
            line.push(format!("{spot}{t}"));
        }
        println!("{row:2}: {}", line.join(" "));
    }
    for a in &state.agents {
        println!(
            "agent {:?} at ({},{}) fuel={} cap={}",
            a.kind, a.pos.col, a.pos.row, a.fuel, a.fuel_cap
        );
    }
    Ok(())
}

#[derive(Clone)]
struct Body {
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    mass: f64,
}

const PI: f64 = 3.141592653589793;
const SOLAR_MASS: f64 = 4.0 * PI * PI;
const DAYS: f64 = 365.24;

fn energy(bodies: &[Body]) -> f64 {
    let mut total = 0.0;
    for i in 0..bodies.len() {
        let b = &bodies[i];
        total += 0.5 * b.mass * (b.vx * b.vx + b.vy * b.vy + b.vz * b.vz);
        for c in &bodies[i + 1..] {
            let (dx, dy, dz) = (b.x - c.x, b.y - c.y, b.z - c.z);
            total -= b.mass * c.mass / (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }
    total
}

fn advance(bodies: &mut [Body], dt: f64) {
    let n = bodies.len();
    for i in 0..n {
        for j in i + 1..n {
            let dx = bodies[i].x - bodies[j].x;
            let dy = bodies[i].y - bodies[j].y;
            let dz = bodies[i].z - bodies[j].z;
            let d2 = dx * dx + dy * dy + dz * dz;
            let mag = dt / (d2 * d2.sqrt());
            let mi = bodies[i].mass * mag;
            let mj = bodies[j].mass * mag;
            bodies[i].vx -= dx * mj;
            bodies[i].vy -= dy * mj;
            bodies[i].vz -= dz * mj;
            bodies[j].vx += dx * mi;
            bodies[j].vy += dy * mi;
            bodies[j].vz += dz * mi;
        }
    }
    for b in bodies.iter_mut() {
        b.x += dt * b.vx;
        b.y += dt * b.vy;
        b.z += dt * b.vz;
    }
}

#[rustfmt::skip]
fn main() {
    let body = |x, y, z, vx: f64, vy: f64, vz: f64, mass: f64| Body {
        x, y, z, vx: vx * DAYS, vy: vy * DAYS, vz: vz * DAYS, mass: mass * SOLAR_MASS,
    };
    let mut bodies = vec![
        Body { x: 0.0, y: 0.0, z: 0.0, vx: 0.0, vy: 0.0, vz: 0.0, mass: SOLAR_MASS },
        body(4.84143144246472082945, -1.16032004402742839362, -0.10362204447112311340,
             0.00166007664274403694, 0.00769901118419740425, -0.00006904600169720630,
             0.00095479193842432661),
        body(8.34336671801457167004, 4.12479856412430479603, -0.40352341711432138415,
             -0.00276742510726862411, 0.00499852801234917238, 0.00002304172975737320,
             0.00028588598066613082),
        body(12.89436956213913102, -15.11115140169863110, -0.22330757889265573,
             0.00296460137564761618, 0.00237847173959480950, -0.00002965895685402375,
             0.00004366244043351564),
        body(15.37969711094732300, -25.91931460998796000, 0.17925877295037118,
             0.00268067772490389322, 0.00162824170038242295, -0.00009515922545197159,
             0.00005151389020466115),
    ];
    let (mut px, mut py, mut pz) = (0.0, 0.0, 0.0);
    for b in &bodies {
        px += b.vx * b.mass;
        py += b.vy * b.mass;
        pz += b.vz * b.mass;
    }
    bodies[0].vx = -px / SOLAR_MASS;
    bodies[0].vy = -py / SOLAR_MASS;
    bodies[0].vz = -pz / SOLAR_MASS;

    let steps = std::env::args().count() * 2000000;
    println!("{:?}", energy(&bodies));
    for _ in 0..steps {
        advance(&mut bodies, 0.01);
    }
    println!("{:?}", energy(&bodies));
}

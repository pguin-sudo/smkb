use smkb_platform::RealMonitor;
use smkb_proto::{DisplayGeometry, Edge, Point};

#[derive(Debug, Clone, Copy)]
pub struct Accel {
    pub sensitivity: f64,
}

pub struct PointerIntegrator {
    bounds: DisplayGeometry,
    accel: Accel,
    x: f64,
    y: f64,
}

impl PointerIntegrator {
    pub fn new(bounds: DisplayGeometry, accel: Accel) -> Self {
        Self { bounds, accel, x: 0.0, y: 0.0 }
    }

    pub fn reset_at(&mut self, p: Point) {
        self.x = p.x as f64;
        self.y = p.y as f64;
    }

    pub fn position(&self) -> Point {
        Point::new(self.x.round() as i32, self.y.round() as i32)
    }

    pub fn apply_delta(&mut self, dx: i32, dy: i32) -> (Point, Option<Edge>) {
        self.x += dx as f64 * self.accel.sensitivity;
        self.y += dy as f64 * self.accel.sensitivity;

        let max_x = self.bounds.width.saturating_sub(1) as f64;
        let max_y = self.bounds.height.saturating_sub(1) as f64;

        let x_over = if self.x < 0.0 { Some((Edge::Left, -self.x)) } else if self.x > max_x { Some((Edge::Right, self.x - max_x)) } else { None };
        let y_over = if self.y < 0.0 { Some((Edge::Top, -self.y)) } else if self.y > max_y { Some((Edge::Bottom, self.y - max_y)) } else { None };
        let edge = match (x_over, y_over) {
            (Some((ex, ox)), Some((ey, oy))) => Some(if ox >= oy { ex } else { ey }),
            (Some((e, _)), None) => Some(e),
            (None, Some((e, _))) => Some(e),
            (None, None) => None,
        };

        self.x = self.x.clamp(0.0, max_x);
        self.y = self.y.clamp(0.0, max_y);

        (self.position(), edge)
    }
}

pub fn at_trigger_edge(pos: Point, monitor: &RealMonitor, side: Edge) -> bool {
    let within_x = pos.x >= monitor.x && pos.x < monitor.x + monitor.width as i32;
    let within_y = pos.y >= monitor.y && pos.y < monitor.y + monitor.height as i32;
    match side {
        Edge::Right => within_y && pos.x >= monitor.x + monitor.width as i32 - 1,
        Edge::Left => within_y && pos.x <= monitor.x,
        Edge::Bottom => within_x && pos.y >= monitor.y + monitor.height as i32 - 1,
        Edge::Top => within_x && pos.y <= monitor.y,
    }
}

fn frac_along(value: i32, origin: i32, span: u32) -> f64 {
    (value - origin) as f64 / span.max(1) as f64
}

pub fn entry_point(pos: Point, monitor: &RealMonitor, side: Edge, slave: DisplayGeometry) -> Point {
    let max_x = slave.width.saturating_sub(1) as i32;
    let max_y = slave.height.saturating_sub(1) as i32;
    match side {
        Edge::Right => {
            let frac = frac_along(pos.y, monitor.y, monitor.height.saturating_sub(1)).clamp(0.0, 1.0);
            Point::new(2, (frac * max_y as f64).round() as i32)
        }
        Edge::Left => {
            let frac = frac_along(pos.y, monitor.y, monitor.height.saturating_sub(1)).clamp(0.0, 1.0);
            Point::new(max_x - 2, (frac * max_y as f64).round() as i32)
        }
        Edge::Bottom => {
            let frac = frac_along(pos.x, monitor.x, monitor.width.saturating_sub(1)).clamp(0.0, 1.0);
            Point::new((frac * max_x as f64).round() as i32, 2)
        }
        Edge::Top => {
            let frac = frac_along(pos.x, monitor.x, monitor.width.saturating_sub(1)).clamp(0.0, 1.0);
            Point::new((frac * max_x as f64).round() as i32, max_y - 2)
        }
    }
}

pub fn exit_point(local_exit: Point, monitor: &RealMonitor, side: Edge, slave: DisplayGeometry) -> Point {
    match side {
        Edge::Right => {
            let frac = frac_along(local_exit.y, 0, slave.height.saturating_sub(1)).clamp(0.0, 1.0);
            let y = monitor.y + (frac * monitor.height.saturating_sub(1) as f64).round() as i32;
            Point::new(monitor.x + monitor.width as i32 - 2, y)
        }
        Edge::Left => {
            let frac = frac_along(local_exit.y, 0, slave.height.saturating_sub(1)).clamp(0.0, 1.0);
            let y = monitor.y + (frac * monitor.height.saturating_sub(1) as f64).round() as i32;
            Point::new(monitor.x + 2, y)
        }
        Edge::Bottom => {
            let frac = frac_along(local_exit.x, 0, slave.width.saturating_sub(1)).clamp(0.0, 1.0);
            let x = monitor.x + (frac * monitor.width.saturating_sub(1) as f64).round() as i32;
            Point::new(x, monitor.y + monitor.height as i32 - 2)
        }
        Edge::Top => {
            let frac = frac_along(local_exit.x, 0, slave.width.saturating_sub(1)).clamp(0.0, 1.0);
            let x = monitor.x + (frac * monitor.width.saturating_sub(1) as f64).round() as i32;
            Point::new(x, monitor.y + 2)
        }
    }
}

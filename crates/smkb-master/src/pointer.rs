use smkb_platform::RealMonitor;
use smkb_proto::{DisplayGeometry, Edge, Point};

pub struct PointerIntegrator {
    bounds: DisplayGeometry,
    sensitivity: f64,
    x: f64,
    y: f64,
}

impl PointerIntegrator {
    pub fn new(bounds: DisplayGeometry, sensitivity: f64) -> Self {
        Self {
            bounds,
            sensitivity,
            x: 0.0,
            y: 0.0,
        }
    }

    pub fn reset_at(&mut self, p: Point) {
        self.x = p.x as f64;
        self.y = p.y as f64;
    }

    pub fn position(&self) -> Point {
        Point::new(self.x.round() as i32, self.y.round() as i32)
    }

    pub fn apply_delta(&mut self, dx: i32, dy: i32) -> (Point, Option<Edge>) {
        self.x += dx as f64 * self.sensitivity;
        self.y += dy as f64 * self.sensitivity;

        let max_x = self.bounds.width.saturating_sub(1) as f64;
        let max_y = self.bounds.height.saturating_sub(1) as f64;

        let x_over = if self.x < 0.0 {
            Some((Edge::Left, -self.x))
        } else if self.x > max_x {
            Some((Edge::Right, self.x - max_x))
        } else {
            None
        };
        let y_over = if self.y < 0.0 {
            Some((Edge::Top, -self.y))
        } else if self.y > max_y {
            Some((Edge::Bottom, self.y - max_y))
        } else {
            None
        };
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

fn map_coordinate(value: i32, origin: i32, span: u32, target_span: f64) -> i32 {
    let fraction = ((value - origin) as f64 / span.max(1) as f64).clamp(0.0, 1.0);
    (fraction * target_span).round() as i32
}

pub fn entry_point(pos: Point, monitor: &RealMonitor, side: Edge, slave: DisplayGeometry) -> Point {
    let max_x = slave.width.saturating_sub(1);
    let max_y = slave.height.saturating_sub(1);
    match side {
        Edge::Right | Edge::Left => {
            let y = map_coordinate(
                pos.y,
                monitor.y,
                monitor.height.saturating_sub(1),
                max_y as i32 as f64,
            );
            Point::new(
                if side == Edge::Right {
                    2
                } else {
                    max_x as i32 - 2
                },
                y,
            )
        }
        Edge::Bottom | Edge::Top => {
            let x = map_coordinate(
                pos.x,
                monitor.x,
                monitor.width.saturating_sub(1),
                max_x as i32 as f64,
            );
            Point::new(
                x,
                if side == Edge::Bottom {
                    2
                } else {
                    max_y as i32 - 2
                },
            )
        }
    }
}

pub fn exit_point(
    local_exit: Point,
    monitor: &RealMonitor,
    side: Edge,
    slave: DisplayGeometry,
) -> Point {
    match side {
        Edge::Right | Edge::Left => {
            let y = monitor.y
                + map_coordinate(
                    local_exit.y,
                    0,
                    slave.height.saturating_sub(1),
                    monitor.height.saturating_sub(1) as f64,
                );
            let x = monitor.x
                + if side == Edge::Right {
                    monitor.width as i32 - 2
                } else {
                    2
                };
            Point::new(x, y)
        }
        Edge::Bottom | Edge::Top => {
            let x = monitor.x
                + map_coordinate(
                    local_exit.x,
                    0,
                    slave.width.saturating_sub(1),
                    monitor.width.saturating_sub(1) as f64,
                );
            let y = monitor.y
                + if side == Edge::Bottom {
                    monitor.height as i32 - 2
                } else {
                    2
                };
            Point::new(x, y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> RealMonitor {
        RealMonitor {
            name: "test".into(),
            x: -100,
            y: 50,
            width: 101,
            height: 201,
            focused: true,
        }
    }

    fn screen() -> DisplayGeometry {
        DisplayGeometry {
            width: 401,
            height: 601,
            refresh_mhz: 60000,
            scale_pct: 100,
        }
    }

    #[test]
    fn doorway_coordinates_for_every_side() {
        let monitor = monitor();
        for (side, entry, exit) in [
            (Edge::Right, Point::new(2, 300), Point::new(-1, 150)),
            (Edge::Left, Point::new(398, 300), Point::new(-98, 150)),
            (Edge::Bottom, Point::new(200, 2), Point::new(-50, 249)),
            (Edge::Top, Point::new(200, 598), Point::new(-50, 52)),
        ] {
            assert_eq!(
                entry_point(Point::new(-50, 150), &monitor, side, screen()),
                entry
            );
            assert_eq!(
                exit_point(Point::new(200, 300), &monitor, side, screen()),
                exit
            );
        }
    }

    #[test]
    fn coordinates_clamp_and_handle_zero_spans() {
        assert_eq!(map_coordinate(-1, 0, 100, 200.0), 0);
        assert_eq!(map_coordinate(101, 0, 100, 200.0), 200);
        assert_eq!(map_coordinate(0, 0, 0, 0.0), 0);
        assert_eq!(map_coordinate(1, 0, 0, 200.0), 200);
    }

    #[test]
    fn motion_preserves_fractional_steps_and_prefers_larger_overflow() {
        let mut pointer = PointerIntegrator::new(screen(), 0.5);
        pointer.reset_at(Point::new(200, 300));
        assert_eq!(pointer.apply_delta(1, 1), (Point::new(201, 301), None));
        assert_eq!(pointer.apply_delta(1, 1), (Point::new(201, 301), None));
        assert_eq!(
            pointer.apply_delta(-1000, -2000),
            (Point::new(0, 0), Some(Edge::Top))
        );
        assert_eq!(
            pointer.apply_delta(-2, -2),
            (Point::new(0, 0), Some(Edge::Left))
        );
    }
}

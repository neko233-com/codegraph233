pub struct Player { hp: i32 }

pub trait Hitter { fn hit(&mut self, d: i32) -> i32; }

impl Hitter for Player {
    fn hit(&mut self, d: i32) -> i32 {
        self.damage(d);
        self.hp
    }
}

impl Player {
    pub fn damage(&mut self, d: i32) { self.hp -= d; }
}

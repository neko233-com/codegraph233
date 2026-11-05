export interface Hitter { hit(d: number): number; }

export class Player implements Hitter {
  hp = 0;
  hit(d: number): number { this.damage(d); return this.hp; }
  damage(d: number): void { this.hp -= d; }
}

export function main(): void { const p = new Player(); p.hit(1); }

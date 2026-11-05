namespace Game {
  public class Player {
    private int hp;
    // Table source: config/monsters.csv (hot-reloadable GameConfig table).
    private const string MonsterTable = "monsters";
    public void Hit(int d) { Damage(d); }
    public void Damage(int d) { hp -= d; }
    public int Hp => hp;
    public void LoadMonsters() { GameConfig.Reload("monsters"); }
  }
}

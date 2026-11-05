namespace Game {
  public class Player {
    private int hp;
    public void Hit(int d) { Damage(d); }
    public void Damage(int d) { hp -= d; }
    public int Hp => hp;
  }
}

#pragma once
#include <vector>
#include "entity.h"

namespace game {

struct Vec2 {
    float x = 0.f;
    float y = 0.f;

    Vec2 operator+(const Vec2& o) const { return {x + o.x, y + o.y}; }
    Vec2 operator*(float s)        const { return {x * s,   y * s};   }
    float dot(const Vec2& o)       const { return x * o.x + y * o.y;  }
    float length()                 const;
    Vec2  normalized()             const;
};

struct AABB {
    float x, y, w, h;
    bool overlaps(const AABB& other) const;
    bool contains(float px, float py) const;
};

class RigidBody : public Component {
public:
    explicit RigidBody(float mass = 1.f);

    void update(float dt) override;
    std::string name() const override { return "RigidBody"; }

    void applyForce(Vec2 force);
    void applyImpulse(Vec2 impulse);
    void setVelocity(Vec2 vel) { velocity_ = vel; }
    Vec2 velocity() const { return velocity_; }
    float mass() const { return mass_; }
    void setGravityScale(float s) { gravityScale_ = s; }

private:
    float mass_;
    float gravityScale_ = 1.f;
    Vec2 velocity_{};
    Vec2 acceleration_{};
};

class PhysicsWorld {
public:
    explicit PhysicsWorld(Vec2 gravity = {0.f, -9.81f});

    void step(float dt);
    void addBody(RigidBody* body);
    void removeBody(RigidBody* body);
    void setGravity(Vec2 g) { gravity_ = g; }
    Vec2 gravity() const { return gravity_; }

    bool raycast(Vec2 origin, Vec2 direction, float maxDist, Entity** hitEntity) const;
    std::vector<Entity*> queryAABB(const AABB& region) const;

private:
    Vec2 gravity_;
    std::vector<RigidBody*> bodies_;
    void resolveCollisions();
};

} // namespace game

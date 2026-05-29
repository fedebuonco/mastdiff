#include "physics.h"
#include <cmath>
#include <algorithm>
#include <iostream>

namespace game {

// ── Vec2 ──────────────────────────────────────────────────────────────────

float Vec2::length() const {
    return std::sqrt(x * x + y * y);
}

Vec2 Vec2::normalized() const {
    float len = length();
    if (len < 1e-6f) return {0.f, 0.f};
    return {x / len, y / len};
}

// ── AABB ──────────────────────────────────────────────────────────────────

bool AABB::overlaps(const AABB& other) const {
    return x < other.x + other.w &&
           x + w > other.x       &&
           y < other.y + other.h &&
           y + h > other.y;
}

bool AABB::contains(float px, float py) const {
    return px >= x && px <= x + w && py >= y && py <= y + h;
}

// ── RigidBody ─────────────────────────────────────────────────────────────

RigidBody::RigidBody(float mass) : mass_(mass) {
    if (mass_ <= 0.f) {
        std::cerr << "[Physics] Warning: non-positive mass clamped to 0.001\n";
        mass_ = 0.001f;
    }
}

void RigidBody::update(float dt) {
    velocity_ = velocity_ + acceleration_ * dt;
    acceleration_ = {0.f, 0.f}; // reset each frame; gravity applied by world
}

void RigidBody::applyForce(Vec2 force) {
    acceleration_ = acceleration_ + force * (1.f / mass_);
}

void RigidBody::applyImpulse(Vec2 impulse) {
    velocity_ = velocity_ + impulse * (1.f / mass_);
}

// ── PhysicsWorld ──────────────────────────────────────────────────────────

PhysicsWorld::PhysicsWorld(Vec2 gravity) : gravity_(gravity) {}

void PhysicsWorld::addBody(RigidBody* body) {
    if (body) bodies_.push_back(body);
}

void PhysicsWorld::removeBody(RigidBody* body) {
    bodies_.erase(std::remove(bodies_.begin(), bodies_.end(), body), bodies_.end());
}

void PhysicsWorld::step(float dt) {
    // Apply gravity and integrate
    for (auto* body : bodies_) {
        body->applyForce(gravity_ * body->mass());
        body->update(dt);
    }
    resolveCollisions();
}

void PhysicsWorld::resolveCollisions() {
    // Placeholder: broad-phase AABB check would go here
    // For each overlapping pair, compute and apply collision response
}

bool PhysicsWorld::raycast(Vec2 origin, Vec2 direction, float maxDist,
                           Entity** hitEntity) const {
    // Simplified ray march
    Vec2 dir = direction.normalized();
    constexpr int steps = 64;
    float stepLen = maxDist / steps;

    for (int i = 0; i < steps; ++i) {
        Vec2 point = origin + dir * (stepLen * i);
        // In a real impl, test against each body's AABB
        (void)point;
    }

    if (hitEntity) *hitEntity = nullptr;
    return false;
}

std::vector<Entity*> PhysicsWorld::queryAABB(const AABB& region) const {
    std::vector<Entity*> results;
    // Query spatial structure for entities in region
    (void)region;
    return results;
}

} // namespace game

#pragma once
#include <string>
#include <vector>
#include <memory>

namespace game {

struct Transform {
    float x = 0.f, y = 0.f, z = 0.f;
    float scale = 1.f;
    float rotation = 0.f;
};

class Component {
public:
    virtual ~Component() = default;
    virtual void update(float dt) = 0;
    virtual std::string name() const = 0;
};

class Entity {
public:
    explicit Entity(std::string id);
    ~Entity();

    void update(float dt);
    void addComponent(std::shared_ptr<Component> comp);
    void removeComponent(const std::string& compName);
    Component* getComponent(const std::string& compName) const;

    const std::string& id() const { return id_; }
    const Transform& transform() const { return transform_; }
    Transform& transform() { return transform_; }

    bool isActive() const { return active_; }
    void setActive(bool v) { active_ = v; }

private:
    std::string id_;
    Transform transform_;
    std::vector<std::shared_ptr<Component>> components_;
    bool active_ = true;
};

class EntityManager {
public:
    Entity* create(const std::string& id);
    void destroy(const std::string& id);
    Entity* find(const std::string& id) const;
    void updateAll(float dt);
    std::size_t count() const;

private:
    std::vector<std::unique_ptr<Entity>> entities_;
};

} // namespace game

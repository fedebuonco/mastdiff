#include <iostream>
#include <vector>
#include <string>

class Animal {
public:
    Animal(const std::string& name, int age, const std::string& species)
        : name_(name), age_(age), species_(species) {}

    virtual std::string speak() const {
        return "...";
    }

    void printInfo() const {
        std::cout << "[" << species_ << "] " << name_ << " (age " << age_ << "): " << speak() << "\n";
    }

    int getAge() const { return age_; }
    std::string getSpecies() const { return species_; }

private:
    std::string name_;
    int age_;
    std::string species_;
};

class Dog : public Animal {
public:
    Dog(const std::string& name, int age)
        : Animal(name, age, "Canis lupus familiaris") {}

    std::string speak() const override {
        return "Woof! Woof!";
    }

    void fetch(const std::string& item) {
        std::cout << "Fetching " << item << " with enthusiasm!\n";
    }

    void sit() {
        std::cout << "Sitting!\n";
    }
};

double computeAverage(const std::vector<int>& nums) {
    if (nums.empty()) return 0.0;
    double sum = 0.0;
    for (int n : nums) {
        sum += n;
    }
    return sum / nums.size();
}

int main() {
    Dog dog("Rex", 3);
    dog.printInfo();
    dog.fetch("ball");
    dog.sit();

    std::vector<int> data = {1, 2, 3, 4, 5};
    std::cout << "Average: " << computeAverage(data) << "\n";

    return 0;
}

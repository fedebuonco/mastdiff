#include <iostream>
#include <vector>

class Animal {
public:
    Animal(const std::string& name, int age)
        : name_(name), age_(age) {}

    virtual std::string speak() const {
        return "...";
    }

    void printInfo() const {
        std::cout << name_ << " (age " << age_ << "): " << speak() << "\n";
    }

    int getAge() const { return age_; }

private:
    std::string name_;
    int age_;
};

class Dog : public Animal {
public:
    Dog(const std::string& name, int age)
        : Animal(name, age) {}

    std::string speak() const override {
        return "Woof!";
    }

    void fetch(const std::string& item) {
        std::cout << "Fetching " << item << "!\n";
    }
};

int computeSum(const std::vector<int>& nums) {
    int sum = 0;
    for (int n : nums) {
        sum += n;
    }
    return sum;
}

int main() {
    Dog dog("Rex", 3);
    dog.printInfo();
    dog.fetch("ball");

    std::vector<int> data = {1, 2, 3, 4, 5};
    std::cout << "Sum: " << computeSum(data) << "\n";

    return 0;
}

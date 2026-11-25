# import section
from math import sqrt

# information
print("*** Calculating the BMI (body mass index) ***\n")

# data input
weight = float(input("Please enter a weight [kg]: "))
height = float(input("Please enter a height [m]: "))

print("\nThank You!\n")

# processing
bmi = weight / height ** 2

if bmi < 18.5:
    status = "underweight."
elif bmi < 25:
    status = "normal weight."
elif bmi < 30:
    status = "overweight."
else:
    status = "obese."

# data output
print(f"The BMI of a height of {height:.2f} m and a weight of {weight:.2f} kg is {bmi:.2f}.")
print("The resulting weight status is", status)

# the end
input("\nPlease hit ENTER to quit...")
